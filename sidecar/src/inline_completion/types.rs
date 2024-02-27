use std::pin::Pin;
use std::sync::Arc;

use chrono::Local;
use futures::{stream, StreamExt};
use futures::{FutureExt, Stream};
use llm_client::{
    broker::LLMBroker,
    clients::types::{LLMClientCompletionStringRequest, LLMType},
    tokenizer::tokenizer::{LLMTokenizer, LLMTokenizerError},
};
use llm_prompts::{
    answer_model::LLMAnswerModelBroker,
    fim::types::{FillInMiddleBroker, FillInMiddleRequest},
};
use tree_sitter::TreeCursor;

use crate::chunking::languages::TSLanguageConfig;
use crate::chunking::text_document::Range;
use crate::inline_completion::context::clipboard_context::{
    ClipboardContext, ClipboardContextString,
};
use crate::{
    chunking::editor_parsing::EditorParsing,
    webserver::inline_completion::{
        InlineCompletion, InlineCompletionRequest, InlineCompletionResponse,
    },
};

use super::{
    context::{current_file::CurrentFileContext, types::DocumentLines},
    helpers::insert_range,
};

#[derive(Debug, Clone)]
pub struct FillInMiddleError {
    error_count: usize,
    missing_count: usize,
}

impl FillInMiddleError {
    pub fn new(error_count: usize, missing_count: usize) -> Self {
        Self {
            error_count,
            missing_count,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FillInMiddleStreamContext {
    file_path: String,
    prefix_at_cursor_position: String,
    document_prefix: String,
    document_suffix: String,
    editor_parsing: Arc<EditorParsing>,
    errors: Option<FillInMiddleError>,
}

impl FillInMiddleStreamContext {
    fn new(
        file_path: String,
        prefix_at_cursor_position: String,
        document_prefix: String,
        document_suffix: String,
        editor_parsing: Arc<EditorParsing>,
        errors: Option<FillInMiddleError>,
    ) -> Self {
        Self {
            file_path,
            prefix_at_cursor_position,
            document_prefix,
            document_suffix,
            editor_parsing,
            errors,
        }
    }
}

pub struct FillInMiddleCompletionAgent {
    llm_broker: Arc<LLMBroker>,
    llm_tokenizer: Arc<LLMTokenizer>,
    fill_in_middle_broker: Arc<FillInMiddleBroker>,
    editor_parsing: Arc<EditorParsing>,
    answer_mode: Arc<LLMAnswerModelBroker>,
}

#[derive(thiserror::Error, Debug)]
pub enum InLineCompletionError {
    #[error("LLM type {0} is not supported for inline completion.")]
    LLMNotSupported(LLMType),

    #[error("Language Not supported: {0}")]
    LanguageNotSupported(String),

    #[error("tokenizer formatting error: {0}")]
    LLMTokenizerError(#[from] llm_client::format::types::TokenizerError),

    #[error("tokenizer error: {0}")]
    LLMTokenizationError(#[from] LLMTokenizerError),

    #[error("No language configuration found for path: {0}")]
    NoLanguageConfiguration(String),

    #[error("Fill in middle error: {0}")]
    FillInMiddleError(#[from] llm_prompts::fim::types::FillInMiddleError),

    #[error("Missing provider keys: {0}")]
    MissingProviderKeys(LLMType),

    #[error("LLMClient error: {0}")]
    LLMClientError(#[from] llm_client::clients::types::LLMClientError),

    #[error("terminated streamed completion")]
    InlineCompletionTerminated,

    #[error("Tokenizer not found: {0}")]
    TokenizerNotFound(LLMType),

    #[error("Tokenization error: {0}")]
    TokenizationError(LLMType),

    #[error("Prefix not found for the cursor position")]
    PrefixNotFound,

    #[error("Suffix not found for cursor position")]
    SuffixNotFound,
}

impl FillInMiddleCompletionAgent {
    pub fn new(
        llm_broker: Arc<LLMBroker>,
        llm_tokenizer: Arc<LLMTokenizer>,
        answer_mode: Arc<LLMAnswerModelBroker>,
        fill_in_middle_broker: Arc<FillInMiddleBroker>,
        editor_parsing: Arc<EditorParsing>,
    ) -> Self {
        Self {
            llm_broker,
            llm_tokenizer,
            answer_mode,
            fill_in_middle_broker,
            editor_parsing,
        }
    }

    pub fn completion(
        &self,
        completion_request: InlineCompletionRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<InlineCompletionResponse, InLineCompletionError>> + Send>>,
        InLineCompletionError,
    > {
        // Now that we have the position, we want to create the request for the fill
        // in the middle request.
        let model_config = &completion_request.model_config;
        let fast_model = model_config.fast_model.clone();
        let temperature = model_config
            .fast_model_temperature()
            .ok_or(InLineCompletionError::LLMNotSupported(fast_model.clone()))?;
        let fast_model_api_key = model_config
            .provider_for_fast_model()
            .ok_or(InLineCompletionError::MissingProviderKeys(
                fast_model.clone(),
            ))?
            .clone();
        let model_config = self.answer_mode.get_answer_model(&fast_model);
        if let None = model_config {
            return Err(InLineCompletionError::LLMNotSupported(fast_model));
        }
        let token_limit = model_config
            .expect("if let None to hold")
            .inline_completion_tokens;
        if let None = token_limit {
            return Err(InLineCompletionError::LLMNotSupported(fast_model));
        }
        let mut token_limit = token_limit.expect("if let None to hold");

        let document_lines = DocumentLines::from_file_content(&completion_request.text);

        let mut prefix = None;
        if let Some(completion_context) = completion_request.cliboard_content {
            let clipboard_context = ClipboardContext::new(
                completion_context,
                self.llm_tokenizer.clone(),
                fast_model.clone(),
                self.editor_parsing.clone(),
                completion_request.filepath.to_owned(),
            )
            .get_clipboard_context(100)?;
            match clipboard_context {
                ClipboardContextString::TruncatedToLimit(
                    clipboard_context,
                    clipboard_tokens_used,
                ) => {
                    token_limit = token_limit - clipboard_tokens_used;
                    prefix = Some(clipboard_context);
                }
                _ => {}
            }
        };

        // Grab the error and missing values from tree-sitter
        let errors = grab_errors_using_tree_sitter(
            self.editor_parsing.clone(),
            &completion_request.text,
            &completion_request.filepath,
        )
        .map(|(error, missing)| FillInMiddleError::new(error, missing));

        // Now we are going to grab the current line prefix
        let cursor_prefix = Arc::new(FillInMiddleStreamContext::new(
            completion_request.filepath.to_owned(),
            document_lines.prefix_at_line(completion_request.position)?,
            document_lines.document_prefix(completion_request.position)?,
            document_lines.document_suffix(completion_request.position)?,
            self.editor_parsing.clone(),
            errors,
        ));

        // Now we generate the prefix and the suffix here
        let completion_context = CurrentFileContext::new(
            completion_request.filepath,
            completion_request.position,
            token_limit as usize,
            self.llm_tokenizer.clone(),
            self.editor_parsing.clone(),
            fast_model.clone(),
        )
        .generate_context(&document_lines)?;

        let formatted_string = self.fill_in_middle_broker.format_context(
            match prefix {
                Some(prefix) => FillInMiddleRequest::new(
                    format!(
                        "{}\n{}",
                        prefix,
                        completion_context.prefix.content().to_owned()
                    ),
                    completion_context.suffix.content().to_owned(),
                ),
                None => FillInMiddleRequest::new(
                    completion_context.prefix.content().to_owned(),
                    completion_context.suffix.content().to_owned(),
                ),
            },
            &fast_model,
        )?;

        let arced_document_lines = Arc::new(document_lines);

        // Now we send a request over to our provider and get a response for this
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let completion_receiver_stream =
            tokio_stream::wrappers::UnboundedReceiverStream::new(receiver).map(either::Left);
        // pin_mut!(merged_stream);

        let llm_broker = self.llm_broker.clone();
        let should_end_stream = Arc::new(std::sync::Mutex::new(false));
        Ok(Box::pin({
            let cursor_prefix = cursor_prefix.clone();
            let should_end_stream = should_end_stream.clone();
            // ugly, ugly, ugly, but type-safe so yay :))
            let completion = LLMBroker::stream_string_completion_owned(
                llm_broker,
                fast_model_api_key,
                LLMClientCompletionStringRequest::new(
                    fast_model.clone(),
                    formatted_string.filled.to_owned(),
                    temperature,
                    None,
                )
                // we are dumping the same eot for different models here, which
                // is fine but we can change this later
                .set_stop_words(vec![
                    "\n\n".to_owned(),
                    "```".to_owned(),
                    "<EOT>".to_owned(),
                    "</s>".to_owned(),
                    "<｜end▁of▁sentence｜>".to_owned(),
                    "<｜begin▁of▁sentence｜>".to_owned(),
                    "<step>".to_owned(),
                ])
                // we only allow for 256 tokens so we can quickly get back the response
                // and terminate if we are going through a bad request
                .set_max_tokens(256),
                vec![("event_type".to_owned(), "fill_in_middle".to_owned())]
                    .into_iter()
                    .collect(),
                sender,
            )
            .into_stream()
            .map(either::Right);

            let merged_stream = stream::select(completion_receiver_stream, completion);
            merged_stream
                .map(move |item| {
                    (
                        item,
                        arced_document_lines.clone(),
                        formatted_string.clone(),
                        cursor_prefix.clone(),
                        should_end_stream.clone(),
                    )
                })
                .map(
                    move |(
                        item,
                        document_lines,
                        formatted_string,
                        cursor_prefix,
                        should_end_stream,
                    )| match item {
                        either::Left(response) => Ok((
                            InlineCompletionResponse::new(
                                vec![InlineCompletion::new(
                                    response.answer_up_until_now().to_owned(),
                                    insert_range(
                                        completion_request.position,
                                        &document_lines,
                                        response.answer_up_until_now(),
                                    ),
                                    response.delta().map(|v| v.to_owned()),
                                )],
                                formatted_string.filled.to_owned(),
                            ),
                            cursor_prefix.clone(),
                            should_end_stream.clone(),
                        )),
                        either::Right(Ok(response)) => {
                            Ok((
                                InlineCompletionResponse::new(
                                    // this gets sent at the very end
                                    vec![InlineCompletion::new(
                                        response.to_owned(),
                                        insert_range(
                                            completion_request.position,
                                            &document_lines,
                                            &response,
                                        ),
                                        None,
                                    )],
                                    formatted_string.filled.to_owned(),
                                ),
                                cursor_prefix,
                                should_end_stream.clone(),
                            ))
                        }
                        either::Right(Err(e)) => {
                            println!("{:?}", e);
                            Err(InLineCompletionError::InlineCompletionTerminated)
                        }
                    },
                )
                // this is used to decide the termination of the stream
                .take_while(
                    |inline_completion_response| match inline_completion_response {
                        Ok((inline_completion_response, cursor_prefix, should_end_stream)) => {
                            // Now we can check if we should still be sending the item over, and we work independently over here on a state
                            // basis and not the stream basis
                            {
                                // we are going ot early bail here if we have reached the end of the stream
                                if let Ok(value) = should_end_stream.lock() {
                                    if *value {
                                        return futures::future::ready(false);
                                    }
                                }
                            }
                            let inserted_text = inline_completion_response
                                .completions
                                .get(0)
                                .map(|completion| completion.insert_text.to_owned());
                            let inserted_range = inline_completion_response
                                .completions
                                .get(0)
                                .map(|completion| completion.insert_range.clone());
                            match (inserted_text, inserted_range) {
                                (Some(inserted_text), Some(inserted_range)) => {
                                    if check_terminating_condition(
                                        inserted_text,
                                        &inserted_range,
                                        cursor_prefix.clone(),
                                    ) {
                                        if let Ok(mut value) = should_end_stream.lock() {
                                            *value = true;
                                        }
                                    }
                                    futures::future::ready(true)
                                }
                                _ => futures::future::ready(true),
                            }
                        }
                        Err(_) => futures::future::ready(false),
                    },
                )
                .map(|item| match item {
                    Ok((inline_completion, _cursor_prefix, _should_end_stream)) => {
                        Ok(inline_completion)
                    }
                    Err(e) => Err(e),
                })
        }))
    }
}

fn check_terminating_condition(
    inserted_text: String,
    inserted_range: &Range,
    context: Arc<FillInMiddleStreamContext>,
) -> bool {
    let final_completion_from_prefix =
        context.prefix_at_cursor_position.to_owned() + &inserted_text;

    let language_config = context.editor_parsing.for_file_path(&context.file_path);

    // TODO(skcd): One of the bugs here is that we could have a closing bracket or something
    // in the suffix, on the editor side this is taken care of automagically,
    // but here we need to take care of it
    // we can either do tree-sitter based termination or based on indentation as well
    // this will help us understand if we can give the user sustainable replies
    // another condition we can add here is to only check this when we have a multiline completion
    // this way we can avoid unnecessary computation
    let inserted_text_lines_length = inserted_text.lines().into_iter().collect::<Vec<_>>().len();
    if inserted_text_lines_length <= 1 {
        return false;
    }
    if let Some(language_config) = language_config {
        let terminating_condition_errors = check_terminating_condition_by_comparing_errors(
            &language_config,
            &context.document_prefix,
            &context.document_suffix,
            &inserted_text,
            inserted_range,
            context.errors.clone(),
        );
        // we need to call the tree-sitter based termination here
        let terminating_condition = check_terminating_condition_tree_sitter(
            &language_config,
            &context.document_prefix,
            &context.document_suffix,
            &inserted_text,
            inserted_range,
        );
        if terminating_condition_errors {
            return true;
        } else {
            return false;
        }
    }

    // first we check if the lines are, and check for opening and closing brackets
    // the patterns we will look for are: {}, [], (), <>
    let opening_brackets = vec!["{"];
    let closing_brackets = vec!["}"];
    // let opening_brackets = vec!["{", "[", "(", "<"];
    // let closing_brackets = vec!["}", "]", ")", ">"];
    let mut bracket_count = 0;
    let mut opening_bracket_detected = false;
    final_completion_from_prefix
        .chars()
        .into_iter()
        .for_each(|character| {
            let character_str = character.to_string();
            if opening_brackets.contains(&character_str.as_str()) {
                bracket_count = bracket_count + 1;
                opening_bracket_detected = true;
            }
            if closing_brackets.contains(&&character_str.as_str()) {
                bracket_count = bracket_count - 1;
            }
        });
    if opening_bracket_detected && bracket_count == 0 {
        true
    } else {
        false
    }
}

fn grab_errors_using_tree_sitter(
    editor_parsing: Arc<EditorParsing>,
    file_content: &str,
    file_path: &str,
) -> Option<(usize, usize)> {
    let language_config = editor_parsing.for_file_path(file_path);
    if let Some(language_config) = language_config {
        let mut parser = tree_sitter::Parser::new();
        let grammar = language_config.grammar;
        let _ = parser.set_language(grammar());
        let tree = parser.parse(file_content.as_bytes(), None);
        if let Some(tree) = tree {
            let mut cursor = tree.walk();
            Some(walk_tree_for_errors_and_missing(&mut cursor))
        } else {
            None
        }
    } else {
        None
    }
}

fn walk_tree_for_errors_and_missing(cursor: &mut TreeCursor) -> (usize, usize) {
    let mut missing = 0;
    let mut error = 0;
    loop {
        let node = cursor.node();

        if node.is_missing() {
            missing = missing + 1;
        }
        if node.is_error() {
            error = error + 1;
        }

        if cursor.goto_first_child() {
            let (error_child, missing_child) = walk_tree_for_errors_and_missing(cursor);
            missing = missing + missing_child;
            error = error + error_child;
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
    (error, missing)
}

fn walk_tree_for_no_errors(cursor: &mut TreeCursor, inserted_range: &Range) -> bool {
    let mut answer = true;
    loop {
        let node = cursor.node();

        fn check_if_inside_range(start_byte: usize, end_byte: usize, inserted_byte: usize) -> bool {
            start_byte <= inserted_byte && inserted_byte <= end_byte
        }

        fn check_if_intersects_range(
            start_byte: usize,
            end_byte: usize,
            inserted_range: &Range,
        ) -> bool {
            check_if_inside_range(start_byte, end_byte, inserted_range.start_byte())
                || check_if_inside_range(start_byte, end_byte, inserted_range.end_byte())
        }

        // First check if the node is in the range or
        // the range of the node intersects with the inserted range
        if check_if_intersects_range(
            node.range().start_byte,
            node.range().end_byte,
            inserted_range,
        ) {
            if node.is_error() || node.is_missing() {
                answer = false;
                return answer;
            }
        }

        if cursor.goto_first_child() {
            answer = answer && walk_tree_for_no_errors(cursor, inserted_range);
            if !answer {
                return answer;
            }
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            return answer;
        }
    }
}

fn check_terminating_condition_tree_sitter(
    language_config: &TSLanguageConfig,
    prefix: &str,
    suffix: &str,
    text_to_insert: &str,
    inserted_range: &Range,
) -> bool {
    let final_document =
        prefix.to_owned() + &insert_string_and_check_suffix(text_to_insert, suffix);
    let grammar = language_config.grammar;
    let mut parser = tree_sitter::Parser::new();
    let _ = parser.set_language(grammar());
    let tree = parser.parse(final_document.as_bytes(), None);
    if let Some(tree) = tree {
        let mut cursor = tree.walk();
        // for termination condition we require there to be no errors
        walk_tree_for_no_errors(&mut cursor, inserted_range)
    } else {
        true
    }
}

fn check_terminating_condition_by_comparing_errors(
    language_config: &TSLanguageConfig,
    prefix: &str,
    suffix: &str,
    text_to_insert: &str,
    inserted_range: &Range,
    previous_errors: Option<FillInMiddleError>,
) -> bool {
    if let None = previous_errors {
        return false;
    }
    let previous_errors = previous_errors.expect("if let None to hold");
    let final_document =
        prefix.to_owned() + &insert_string_and_check_suffix(text_to_insert, suffix);
    let grammar = language_config.grammar;
    let mut parser = tree_sitter::Parser::new();
    let _ = parser.set_language(grammar());
    let tree = parser.parse(final_document.as_bytes(), None);
    if let Some(tree) = tree {
        let mut cursor = tree.walk();
        let (error, missing) = walk_tree_for_errors_and_missing(&mut cursor);
        // Now we are going to check if any of the errors or missing have stayed the same
        // or increased after the insertion, this is important because
        // the user might have typed in `fn add()`
        // this can also introduce errors and when we get the first line we might not have
        // reduced the errors at that point
        if error >= previous_errors.error_count || missing >= previous_errors.missing_count {
            false
        } else {
            true
        }
    } else {
        false
    }
}

/// The condition here is that we might be matching some characters in the suffix
/// which are on the same line as the inserted text
/// imagine you are doing the following:
/// console.log(<cursor_here>)
/// and the completion here is a, b, c)
/// vscode here will show the completion as valid and also match the closing bracket
/// so when joining the string we have to take care of this case on our own
fn insert_string_and_check_suffix(text_to_insert: &str, suffix: &str) -> String {
    // if the suffix does not exist and it starts with a new line, then just go to the next line
    if suffix.starts_with("\n") {
        return text_to_insert.to_owned() + suffix;
    }
    let suffix_lines = suffix
        .lines()
        .into_iter()
        .map(|line| line.to_owned())
        .collect::<Vec<String>>();
    let text_to_insert_lines = text_to_insert
        .lines()
        .into_iter()
        .map(|line| line.to_owned())
        .collect::<Vec<String>>();
    if suffix_lines.is_empty() {
        text_to_insert.to_owned()
    } else if text_to_insert_lines.is_empty() {
        suffix.to_owned()
    } else {
        let suffix_first_line = suffix_lines[0].clone();
        let text_to_insert_first_line = text_to_insert_lines[0].clone();
        // Now we need to match the characters from the suffix line which are also present in the text_to_insert line
        // and then generate the final line over here
        let mut text_to_insert_position = 0;
        let mut suffix_first_line_index = 0;
        while suffix_first_line_index < suffix_first_line.len() {
            if text_to_insert_position >= text_to_insert_first_line.len() {
                break;
            }
            if suffix_first_line.chars().nth(suffix_first_line_index)
                == text_to_insert_first_line
                    .chars()
                    .nth(text_to_insert_position)
            {
                suffix_first_line_index = suffix_first_line_index + 1;
                text_to_insert_position = text_to_insert_position + 1;
            } else {
                text_to_insert_position = text_to_insert_position + 1;
            }
        }
        let remaining_suffix = if suffix_first_line_index < suffix_first_line.len() {
            &suffix_first_line[suffix_first_line_index..]
        } else {
            ""
        };
        // create the new first line here
        let text_to_insert_first_line = text_to_insert_first_line + remaining_suffix;
        // now create the text to insert from the remaining lines
        let text_to_insert = if text_to_insert_lines.len() > 1 {
            text_to_insert_first_line + "\n" + &text_to_insert_lines[1..].join("\n")
        } else {
            text_to_insert_first_line
        };
        let final_text = if suffix_lines.len() > 1 {
            text_to_insert + "\n" + &suffix_lines[1..].join("\n")
        } else {
            text_to_insert
        };
        // Now the total string will look like the following:
        // text_to_insert_first_line + remaining_suffix from first line
        // text_to_insert_rest_of_lines
        // suffix_rest of the lines
        final_text
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{
        chunking::text_document::{Position, Range},
        inline_completion::types::insert_string_and_check_suffix,
    };

    use super::{check_terminating_condition, FillInMiddleStreamContext};

    #[test]
    fn test_check_terminating_condition_for_if() {
        let context = Arc::new(FillInMiddleStreamContext::new(
            "something.ts".to_owned(),
            "if ".to_owned(),
            "something_else".to_owned(),
            "something_else".to_owned(),
            Arc::new(Default::default()),
            None,
        ));
        let inserted_text = "(blah: number) => {".to_owned();
        let inserted_range = Range::new(Position::new(0, 0, 0), Position::new(0, 4, 7));
        assert_eq!(
            check_terminating_condition(inserted_text, &inserted_range, context),
            false
        );
    }

    #[test]
    fn test_check_terminating_condition_for_if_proper() {
        let context = Arc::new(FillInMiddleStreamContext::new(
            "something.ts".to_owned(),
            "if ".to_owned(),
            "something_else".to_owned(),
            "something_else".to_owned(),
            Arc::new(Default::default()),
            None,
        ));
        let inserted_text = "if (blah: number) => {\nconsole.log('blah');}".to_owned();
        let inserted_range = Range::new(Position::new(0, 0, 0), Position::new(0, 4, 7));
        assert_eq!(
            check_terminating_condition(inserted_text, &inserted_range, context),
            true
        );
    }

    #[test]
    fn test_check_insert_string_and_check_suffix() {
        let text_to_insert = "(a, b, c)".to_owned();
        let suffix = ")\nsomething_else".to_owned();
        let final_text = insert_string_and_check_suffix(&text_to_insert, &suffix);
        assert_eq![final_text, "(a, b, c)\nsomething_else"];
    }

    #[test]
    fn test_check_insert_with_pending_brackets() {
        let text_to_insert = "(a, b, c, d){\nconsole.log('blah');}".to_owned();
        let suffix = ")".to_owned();
        let final_text = insert_string_and_check_suffix(&text_to_insert, &suffix);
        assert_eq![final_text, "(a, b, c, d){\nconsole.log('blah');}"];
    }
}