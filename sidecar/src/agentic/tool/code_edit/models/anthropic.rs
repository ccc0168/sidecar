use llm_client::clients::types::{LLMClientCompletionRequest, LLMClientMessage};

use crate::agentic::tool::code_edit::{find::FindCodeSelectionInput, types::CodeEdit};

use super::broker::{CodeEditPromptFormatters, CodeSnippetForEditing};

pub struct AnthropicCodeEditFromatter {}

impl AnthropicCodeEditFromatter {
    pub fn new() -> Self {
        Self {}
    }

    fn system_message(&self, language: &str, file_path: &str) -> String {
        format!(
            r#"You are an expert software engineer who writes the most high quality code without making any mistakes.
Follow the user's requirements carefully and to the letter.
- The user instructions are present in <user_instruction> tag.
- Modify the code or create new code.
- The code present above the section you have to edit will be given in <code_above> section.
- The code present below the section you have to edit will be given in <code_below> section.
- The code you have to rewrite will be given to you in <code_to_edit> section.
- User the additional context provided to you in <extra_data> section to understand the functions avaialable on different types of variables, it might have additional context provided by the user, use them as required.
- The code you have to edit is in {file_path}
- Output the edited code in a single code block.
- Each code block starts with ```{language}.
- You must always answer in {language} code."#
        )
    }

    fn above_selection(&self, above_selection: Option<&str>) -> Option<String> {
        if let Some(above_selection) = above_selection {
            Some(format!(
                r#"I have the following code above:
<code_above>
{above_selection}
</code_above>"#
            ))
        } else {
            None
        }
    }

    fn below_selection(&self, below_selection: Option<&str>) -> Option<String> {
        if let Some(below_selection) = below_selection {
            Some(format!(
                r#"I have the following code below:
<code_below>
{below_selection}
</code_below>"#
            ))
        } else {
            None
        }
    }

    fn selection_to_edit(&self, selection_to_edit: &str) -> String {
        format!(
            r#"I have the following code in selection to edit:
<code_to_edit>
{selection_to_edit}
</code_to_edit>"#
        )
    }

    fn extra_data(&self, extra_data: &str) -> String {
        format!(
            r#"This is the extra data which you can use:
<extra_data>
{extra_data}
</extra_data>"#
        )
    }

    fn system_message_for_code_to_edit(&self) -> String {
        format!("You are an expert software engineer tasked with finding the right code snippets where edits need to be made for satisfying the user request.
You will be given user instructions in the <user_instruction> section, and the file along with the contents in <file> section.
The file has been divided into sections like so:
<file>
<path>some_file_path</path>
<section>
<id>1</id>
<content>
file_content...
</content>
<id>2</id>
<content>
file_content...
</content>
.... more contents
</file>

You have to first think step by step on how the change can be done, and then select the sections of the file where the changes need to be done along with your reasoning.

As an example:
<file>
<path>tests/calculate.py</path>
<section>
<id>1</id>
<content>
import unittest
from calculator import Calculator

class TestAddition(unittest.TestCase):
    def setUp(self):
        self.calc = Calculator()

    def test_add_positive_numbers(self):
        result = self.calc.add(2, 3)
        self.assertEqual(result, 5)

    def test_add_negative_numbers(self):
        result = self.calc.add(-2, -3)
        self.assertEqual(result, -5)

    def test_add_zero(self):
        result = self.calc.add(0, 0)
        self.assertEqual(result, 0)

</content>
</section>
<section>
<id>
2
</id>
<content>
class TestSubtraction(unittest.TestCase):
    def setUp(self):
        self.calc = Calculator()

    def test_subtract_positive_numbers(self):
        result = self.calc.subtract(5, 3)
        self.assertEqual(result, 2)

    def test_subtract_negative_numbers(self):
        result = self.calc.subtract(-5, -3)
        self.assertEqual(result, -2)

    def test_subtract_zero(self):
        result = self.calc.subtract(5, 0)
        self.assertEqual(result, 5)

</content>
</section>
<section>
<id>
3
</id>
<content>
class TestMultiplication(unittest.TestCase):
    def setUp(self):
        self.calc = Calculator()

    def test_multiply_positive_numbers(self):
        result = self.calc.multiply(2, 3)
        self.assertEqual(result, 6)

    def test_multiply_negative_numbers(self):
        result = self.calc.multiply(-2, 3)
        self.assertEqual(result, -6)

    def test_multiply_by_zero(self):
        result = self.calc.multiply(5, 0)
        self.assertEqual(result, 0)

</content>
</section>
</file>

<user_instruction>
We are modifying the test case for multiplying 2 positive numbers in the calculator_test.py file.
</user_instruction>

Your reply should be the in the following format:
<reply>
<sections>
<section>
<id>
3
</id>
<thinking>
We need to select this block to edit because this is where the test for multiplying 2 positive numbers is present. 
</thinking>
</section>
</sections>
</reply>
")
    }
}

impl CodeEditPromptFormatters for AnthropicCodeEditFromatter {
    fn format_prompt(&self, context: &CodeEdit) -> LLMClientCompletionRequest {
        let extra_data = self.extra_data(context.extra_content());
        let above = self.above_selection(context.above_context());
        let below = self.below_selection(context.below_context());
        let in_range = self.selection_to_edit(context.code_to_edit());
        let language = context.language();
        let fs_file_path = context.fs_file_path();
        let system_message = self.system_message(language, fs_file_path);
        let mut messages = vec![];

        // add the system message
        messages.push(LLMClientMessage::system(system_message));

        let mut user_message = "".to_owned();
        user_message = user_message + &extra_data + "\n";
        if let Some(above) = above {
            user_message = user_message + &above + "\n";
        }
        if let Some(below) = below {
            user_message = user_message + &below + "\n";
        }
        user_message = user_message + &in_range + "\n";

        // Now we add the instruction from the user
        let user_instruction = context.instruction();
        user_message = user_message
            + &format!(
                r#"Only edit the code in <code_to_edit> section, my instructions are:
<user_instruction>
{user_instruction}
</user_instruction>"#
            );

        // Now add the user message to the messages
        messages.push(LLMClientMessage::user(user_message));
        // we use 0.2 temperature so the model can imagine ✨
        LLMClientCompletionRequest::new(context.model().clone(), messages, 0.2, None)
    }

    fn find_code_section(&self, context: &CodeSnippetForEditing) -> LLMClientCompletionRequest {
        // we might want to either add new code or find the code to edit
        // code to edit might be pretty simple, since we can figure out what needs to be done
        // code to add is tricky because we want to find the code location where we want to place it
        // are we going to send symbols or are we going to send whole code blocks?
        // we can also look at the recently edited line in this file which might get a priority over here
        // we can show that with a + mark across the line for each of use and figuring out
        // how to make changes (excluding the imports which we will fix later on)
        let snippets = context.snippets();
        let file_path = context.file_path();
        let user_instruction = context.user_query();
        let formatted_snippets = snippets
            .into_iter()
            .enumerate()
            .map(|(idx, snippet)| {
                let content = snippet.snippet_content();
                format!(
                    r#"<section>
<id>
{idx}
</id>
<content>
{content}
</content>
</section>"#
                )
                .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let user_message = format!(
            r#"<file>
<path>{file_path}</path>
{formatted_snippets}
</file>

<user_instruction>
{user_instruction}
</user_instruction>"#
        );

        let system_message = self.system_message_for_code_to_edit();
        LLMClientCompletionRequest::new(
            context.model().clone(),
            vec![
                LLMClientMessage::system(system_message),
                LLMClientMessage::user(user_message),
            ],
            0.2,
            None,
        )
    }
}