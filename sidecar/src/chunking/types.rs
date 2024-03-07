use super::text_document::Range;

/// Some common types which can be reused across calls

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct FunctionNodeInformation {
    name: String,
    parameters: String,
    body: String,
    return_type: String,
    documentation: Option<String>,
    variables: Vec<(String, Range)>,
}

impl FunctionNodeInformation {
    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn set_parameters(&mut self, parameters: String) {
        self.parameters = parameters;
    }

    pub fn set_body(&mut self, body: String) {
        self.body = body;
    }

    pub fn set_return_type(&mut self, return_type: String) {
        self.return_type = return_type;
    }

    pub fn set_variable_name(&mut self, variable_name: String, variable_range: Range) {
        self.variables.push((variable_name, variable_range));
    }

    pub fn set_documentation(&mut self, documentation: String) {
        self.documentation = Some(documentation);
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_parameters(&self) -> &str {
        &self.parameters
    }

    pub fn get_return_type(&self) -> &str {
        &self.return_type
    }

    pub fn get_documentation(&self) -> Option<&str> {
        self.documentation.as_deref()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
pub enum OutlineNodeType {
    // The identifier for the complete class body
    Class,
    // the name of the class
    ClassName,
    // the identifier for the complete function body
    Function,
    // the name of the funciton
    FunctionName,
    // the body of the function
    FunctionBody,
}

impl OutlineNodeType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "definition.class" => Some(Self::Class),
            "definition.class.name" => Some(Self::ClassName),
            "definition.function" | "definition.method" => Some(Self::Function),
            "function.name" => Some(Self::FunctionName),
            "function.body" => Some(Self::FunctionBody),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OutlineNodeContent {
    range: Range,
    name: String,
    r#type: OutlineNodeType,
    content: String,
}

impl OutlineNodeContent {
    pub fn new(name: String, range: Range, r#type: OutlineNodeType, content: String) -> Self {
        Self {
            range,
            name,
            r#type,
            content,
        }
    }

    pub fn range(&self) -> &Range {
        &self.range
    }
}

#[derive(Debug, Clone)]
pub struct OutlineNode {
    content: OutlineNodeContent,
    children: Vec<OutlineNodeContent>,
    language: String,
}

impl OutlineNode {
    pub fn new(
        content: OutlineNodeContent,
        children: Vec<OutlineNodeContent>,
        language: String,
    ) -> Self {
        Self {
            content,
            children,
            language,
        }
    }

    pub fn range(&self) -> &Range {
        &self.content.range
    }

    pub fn name(&self) -> &str {
        &self.content.name
    }

    pub fn is_class(&self) -> bool {
        matches!(self.content.r#type, OutlineNodeType::Class)
    }

    pub fn get_outline(&self) -> Option<String> {
        // we want to generate the outline for the node here, we have to do some
        // language specific gating here but thats fine
        match &self.content.r#type {
            OutlineNodeType::Class => {
                if self.children.is_empty() {
                    Some(self.content.content.to_owned())
                } else {
                    // for rust we have a special case here as we might have functions
                    // inside which we want to show but its part of the implementation
                    if &self.language == "rust" {
                        // this is 100% a implementation unless over here, so lets use
                        // it as such
                        let implementation_name = self.content.name.to_owned();
                        let children_content = self
                            .children
                            .iter()
                            .map(|children| children.content.to_owned())
                            .collect::<Vec<_>>()
                            .join("\n");
                        Some(format!(
                            "impl {implementation_name} {{\n{children_content}\n}}"
                        ))
                    } else {
                        // TODO(skcd): We will figure out support for other languages
                        None
                    }
                }
            }
            OutlineNodeType::Function => None,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FunctionNodeType {
    // The identifier for the function
    Identifier,
    // The body of the function without the identifier
    Body,
    // The full function with its name and the body
    Function,
    // The parameters of the function
    Parameters,
    // The return type of the function
    ReturnType,
}

impl FunctionNodeType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "identifier" => Some(Self::Identifier),
            "body" => Some(Self::Body),
            "function" => Some(Self::Function),
            "parameters" => Some(Self::Parameters),
            "return_type" => Some(Self::ReturnType),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FunctionInformation {
    range: Range,
    r#type: FunctionNodeType,
    node_information: Option<FunctionNodeInformation>,
}

impl FunctionInformation {
    pub fn new(range: Range, r#type: FunctionNodeType) -> Self {
        Self {
            range,
            r#type,
            node_information: None,
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.node_information.as_ref().map(|info| info.get_name())
    }

    pub fn get_node_information(&self) -> Option<&FunctionNodeInformation> {
        self.node_information.as_ref()
    }

    pub fn set_node_information(&mut self, node_information: FunctionNodeInformation) {
        self.node_information = Some(node_information);
    }

    pub fn set_documentation(&mut self, documentation: String) {
        if let Some(node_information) = &mut self.node_information {
            node_information.set_documentation(documentation);
        }
    }

    pub fn insert_identifier_node(&mut self, identiifer_name: String, identifier_range: Range) {
        if let Some(node_information) = &mut self.node_information {
            node_information.set_variable_name(identiifer_name, identifier_range);
        }
    }

    pub fn get_identifier_nodes(&self) -> Option<&Vec<(String, Range)>> {
        self.node_information.as_ref().map(|info| &info.variables)
    }

    pub fn range(&self) -> &Range {
        &self.range
    }

    pub fn r#type(&self) -> &FunctionNodeType {
        &self.r#type
    }

    pub fn content(&self, file_content: &str) -> String {
        file_content[self.range().start_byte()..self.range().end_byte()].to_owned()
    }

    pub fn find_function_in_byte_offset<'a>(
        function_blocks: &'a [&'a Self],
        byte_offset: usize,
    ) -> Option<&'a Self> {
        let mut possible_function_block = None;
        for function_block in function_blocks.into_iter() {
            // if the end byte for this block is greater than the current byte
            // position and the start byte is greater than the current bytes
            // position as well, we have our function block
            if !(function_block.range().end_byte() < byte_offset) {
                if function_block.range().start_byte() > byte_offset {
                    break;
                }
                possible_function_block = Some(function_block);
            }
        }
        possible_function_block.copied()
    }

    pub fn get_expanded_selection_range(
        function_bodies: &[&FunctionInformation],
        selection_range: &Range,
    ) -> Range {
        let mut start_position = selection_range.start_position();
        let mut end_position = selection_range.end_position();
        let selection_start_fn_body =
            Self::find_function_in_byte_offset(function_bodies, selection_range.start_byte());
        let selection_end_fn_body =
            Self::find_function_in_byte_offset(function_bodies, selection_range.end_byte());

        // What we are trying to do here is expand our selection to cover the whole
        // function if we have to
        if let Some(selection_start_function) = selection_start_fn_body {
            // check if we can expand the range a bit here
            if start_position.to_byte_offset() > selection_start_function.range().start_byte() {
                start_position = selection_start_function.range().start_position();
            }
            // check if the function block ends after our current selection
            if selection_start_function.range().end_byte() > end_position.to_byte_offset() {
                end_position = selection_start_function.range().end_position();
            }
        }
        if let Some(selection_end_function) = selection_end_fn_body {
            // check if we can expand the start position byte here a bit
            if selection_end_function.range().start_byte() < start_position.to_byte_offset() {
                start_position = selection_end_function.range().start_position();
            }
            if selection_end_function.range().end_byte() > end_position.to_byte_offset() {
                end_position = selection_end_function.range().end_position();
            }
        }
        Range::new(start_position, end_position)
    }

    pub fn fold_function_blocks(mut function_blocks: Vec<Self>) -> Vec<Self> {
        // First we sort the function blocks(which are bodies) based on the start
        // index or the end index
        function_blocks.sort_by(|a, b| {
            a.range()
                .start_byte()
                .cmp(&b.range().start_byte())
                .then_with(|| b.range().end_byte().cmp(&a.range().end_byte()))
        });

        // Now that these are sorted we only keep the ones which are not overlapping
        // or fully contained in the other one
        let mut filtered_function_blocks = Vec::new();
        let mut index = 0;

        while index < function_blocks.len() {
            filtered_function_blocks.push(function_blocks[index].clone());
            let mut iterate_index = index + 1;
            while iterate_index < function_blocks.len()
                && function_blocks[index]
                    .range()
                    .is_contained(&function_blocks[iterate_index].range())
            {
                iterate_index += 1;
            }
            index = iterate_index;
        }

        filtered_function_blocks
    }

    pub fn add_documentation_to_functions(
        mut function_blocks: Vec<Self>,
        documentation_entries: Vec<(Range, String)>,
    ) -> Vec<Self> {
        // First we sort the function blocks based on the start index or the end index
        function_blocks.sort_by(|a, b| {
            a.range()
                .start_byte()
                .cmp(&b.range().start_byte())
                .then_with(|| b.range().end_byte().cmp(&a.range().end_byte()))
        });
        let documentation_entires = concat_documentation_string(documentation_entries);
        // now we want to concat the functions to the documentation strings
        // we will use a 2 pointer approach here and keep track of what the current function is and what the current documentation string is
        function_blocks
            .into_iter()
            .map(|mut function_block| {
                documentation_entires
                    .iter()
                    .for_each(|documentation_entry| {
                        if function_block.range().start_line() != 0
                            && documentation_entry.0.end_line()
                                == function_block.range().start_line() - 1
                        {
                            // we have a documentation entry which is right above the function block
                            // we will add this to the function block
                            function_block.set_documentation(documentation_entry.1.to_owned());
                            // we will also update the function block range to include the documentation entry
                            function_block
                                .range
                                .set_start_position(documentation_entry.0.start_position());
                        }
                    });
                // Here we will look for the documentation entries which are just one line above the function range and add that to the function
                // context and update the function block range
                function_block
            })
            .collect()
    }

    pub fn add_identifier_nodes(
        mut function_blocks: Vec<Self>,
        identifier_nodes: Vec<(String, Range)>,
    ) -> Vec<Self> {
        // First we sort the function blocks based on the start index or the end index
        function_blocks.sort_by(|a, b| {
            a.range()
                .start_byte()
                .cmp(&b.range().start_byte())
                .then_with(|| b.range().end_byte().cmp(&a.range().end_byte()))
        });
        function_blocks
            .into_iter()
            .map(|mut function_block| {
                identifier_nodes.iter().for_each(|identifier_node| {
                    let name = &identifier_node.0;
                    let range = identifier_node.1;
                    if function_block.range().contains(&range) {
                        function_block.insert_identifier_node(name.to_owned(), range);
                    }
                });
                function_block
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClassNodeType {
    Identifier,
    ClassDeclaration,
}

impl ClassNodeType {
    pub fn from_str(s: &str) -> Option<ClassNodeType> {
        match s {
            "identifier" => Some(Self::Identifier),
            "class_declaration" => Some(Self::ClassDeclaration),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClassInformation {
    range: Range,
    name: String,
    class_node_type: ClassNodeType,
    documentation: Option<String>,
}

impl ClassInformation {
    pub fn new(range: Range, name: String, class_node_type: ClassNodeType) -> Self {
        Self {
            range,
            name,
            class_node_type,
            documentation: None,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn get_class_type(&self) -> &ClassNodeType {
        &self.class_node_type
    }

    pub fn range(&self) -> &Range {
        &self.range
    }

    pub fn set_documentation(&mut self, documentation: String) {
        self.documentation = Some(documentation);
    }

    pub fn content(&self, content: &str) -> String {
        content[self.range().start_byte()..self.range().end_byte()].to_string()
    }

    pub fn fold_class_information(mut classes: Vec<Self>) -> Vec<Self> {
        // First we sort the function blocks(which are bodies) based on the start
        // index or the end index
        classes.sort_by(|a, b| {
            a.range()
                .start_byte()
                .cmp(&b.range().start_byte())
                .then_with(|| b.range().end_byte().cmp(&a.range().end_byte()))
        });

        // Now that these are sorted we only keep the ones which are not overlapping
        // or fully contained in the other one
        let mut filtered_classes = Vec::new();
        let mut index = 0;

        while index < classes.len() {
            filtered_classes.push(classes[index].clone());
            let mut iterate_index = index + 1;
            while iterate_index < classes.len()
                && classes[index]
                    .range()
                    .is_contained(&classes[iterate_index].range())
            {
                iterate_index += 1;
            }
            index = iterate_index;
        }

        filtered_classes
    }

    pub fn add_documentation_to_classes(
        mut class_blocks: Vec<Self>,
        documentation_entries: Vec<(Range, String)>,
    ) -> Vec<Self> {
        // First we sort the function blocks based on the start index or the end index
        class_blocks.sort_by(|a, b| {
            a.range()
                .start_byte()
                .cmp(&b.range().start_byte())
                .then_with(|| b.range().end_byte().cmp(&a.range().end_byte()))
        });
        let documentation_entires = concat_documentation_string(documentation_entries);
        // now we want to concat the functions to the documentation strings
        // we will use a 2 pointer approach here and keep track of what the current function is and what the current documentation string is
        class_blocks
            .into_iter()
            .map(|mut class_block| {
                documentation_entires
                    .iter()
                    .for_each(|documentation_entry| {
                        if class_block.range().start_line() != 0
                            && documentation_entry.0.end_line()
                                == class_block.range().start_line() - 1
                        {
                            // we have a documentation entry which is right above the function block
                            // we will add this to the function block
                            class_block.set_documentation(documentation_entry.1.to_owned());
                            // we will also update the function block range to include the documentation entry
                            class_block
                                .range
                                .set_start_position(documentation_entry.0.start_position());
                        }
                    });
                // Here we will look for the documentation entries which are just one line above the function range and add that to the function
                // context and update the function block range
                class_block
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct ClassWithFunctions {
    pub class_information: Option<ClassInformation>,
    pub function_information: Vec<FunctionInformation>,
}

impl ClassWithFunctions {
    pub fn class_functions(
        class_information: ClassInformation,
        function_information: Vec<FunctionInformation>,
    ) -> Self {
        Self {
            class_information: Some(class_information),
            function_information,
        }
    }

    pub fn functions(function_information: Vec<FunctionInformation>) -> Self {
        Self {
            class_information: None,
            function_information,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeNodeType {
    Identifier,
    TypeDeclaration,
}

#[derive(Debug, Clone)]
pub struct TypeInformation {
    pub range: Range,
    pub name: String,
    pub node_type: TypeNodeType,
    pub documentation: Option<String>,
}

impl TypeNodeType {
    pub fn from_str(s: &str) -> Option<TypeNodeType> {
        match s {
            "identifier" => Some(Self::Identifier),
            "type_declaration" => Some(Self::TypeDeclaration),
            _ => None,
        }
    }
}

impl TypeInformation {
    pub fn new(range: Range, name: String, type_node_type: TypeNodeType) -> Self {
        Self {
            range,
            name,
            node_type: type_node_type,
            documentation: None,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn set_documentation(&mut self, documentation: String) {
        self.documentation = Some(documentation);
    }

    pub fn get_type_type(&self) -> &TypeNodeType {
        &self.node_type
    }

    pub fn range(&self) -> &Range {
        &self.range
    }

    pub fn content(&self, content: &str) -> String {
        content[self.range().start_byte()..self.range().end_byte()].to_string()
    }

    pub fn fold_type_information(mut types: Vec<Self>) -> Vec<Self> {
        // First we sort the function blocks(which are bodies) based on the start
        // index or the end index
        types.sort_by(|a, b| {
            a.range()
                .start_byte()
                .cmp(&b.range().start_byte())
                .then_with(|| b.range().end_byte().cmp(&a.range().end_byte()))
        });

        // Now that these are sorted we only keep the ones which are not overlapping
        // or fully contained in the other one
        let mut filtered_types = Vec::new();
        let mut index = 0;

        while index < types.len() {
            filtered_types.push(types[index].clone());
            let mut iterate_index = index + 1;
            while iterate_index < types.len()
                && types[index]
                    .range()
                    .is_contained(&types[iterate_index].range())
            {
                iterate_index += 1;
            }
            index = iterate_index;
        }

        filtered_types
    }

    pub fn add_documentation_to_types(
        mut type_blocks: Vec<Self>,
        documentation_entries: Vec<(Range, String)>,
    ) -> Vec<Self> {
        // First we sort the function blocks based on the start index or the end index
        type_blocks.sort_by(|a, b| {
            a.range()
                .start_byte()
                .cmp(&b.range().start_byte())
                .then_with(|| b.range().end_byte().cmp(&a.range().end_byte()))
        });
        let documentation_entires = concat_documentation_string(documentation_entries);
        // now we want to concat the functions to the documentation strings
        // we will use a 2 pointer approach here and keep track of what the current function is and what the current documentation string is
        type_blocks
            .into_iter()
            .map(|mut type_block| {
                documentation_entires
                    .iter()
                    .for_each(|documentation_entry| {
                        if type_block.range().start_line() != 0
                            && documentation_entry.0.end_line()
                                == type_block.range().start_line() - 1
                        {
                            // we have a documentation entry which is right above the function block
                            // we will add this to the function block
                            type_block.set_documentation(documentation_entry.1.to_owned());
                            // we will also update the function block range to include the documentation entry
                            type_block
                                .range
                                .set_start_position(documentation_entry.0.start_position());
                        }
                    });
                // Here we will look for the documentation entries which are just one line above the function range and add that to the function
                // context and update the function block range
                type_block
            })
            .collect()
    }
}

pub fn concat_documentation_string(
    mut documentation_entries: Vec<(Range, String)>,
) -> Vec<(Range, String)> {
    // we also sort the doucmentation entries based on the start index or the end index
    documentation_entries.sort_by(|a, b| {
        a.0.start_byte()
            .cmp(&b.0.start_byte())
            .then_with(|| b.0.end_byte().cmp(&a.0.end_byte()))
    });
    // We also want to concat the documentation entires if they are right after one another for example:
    // // This is a comment
    // // This is another comment
    // fn foo() {}
    // We want to make sure that we concat the comments into one
    let mut documentation_index = 0;
    let mut concatenated_documentation_queries: Vec<(Range, String)> = Vec::new();
    while documentation_index < documentation_entries.len() {
        let mut iterate_index = documentation_index + 1;
        let mut current_index_end_line = documentation_entries[documentation_index].0.end_line();
        let mut documentation_str = documentation_entries[documentation_index].1.to_owned();
        let mut documentation_range = documentation_entries[documentation_index].0.clone();

        // iterate over consecutive entries in the comments
        while iterate_index < documentation_entries.len()
            && current_index_end_line + 1 == documentation_entries[iterate_index].0.start_line()
        {
            current_index_end_line = documentation_entries[iterate_index].0.end_line();
            documentation_str = documentation_str + "\n" + &documentation_entries[iterate_index].1;
            documentation_range
                .set_end_position(documentation_entries[iterate_index].0.end_position());
            iterate_index += 1;
        }
        concatenated_documentation_queries.push((documentation_range, documentation_str));
        documentation_index = iterate_index;
        // either we hit the end of we have a bunch of documentation entries which are consecutive
        // we know what the comment should be and we can add a new entry
    }
    concatenated_documentation_queries
}

#[cfg(test)]
mod tests {
    use crate::chunking::text_document::Position;
    use crate::chunking::text_document::Range;

    use super::concat_documentation_string;

    #[test]
    fn test_documentation_string_concatenation() {
        let documentation_strings = vec![
            (
                Range::new(Position::new(0, 0, 0), Position::new(0, 0, 0)),
                "first_comment".to_owned(),
            ),
            (
                Range::new(Position::new(1, 0, 0), Position::new(1, 0, 0)),
                "second_comment".to_owned(),
            ),
            (
                Range::new(Position::new(4, 0, 0), Position::new(6, 0, 0)),
                "third_multi_line_comment".to_owned(),
            ),
            (
                Range::new(Position::new(7, 0, 0), Position::new(7, 0, 0)),
                "fourth_comment".to_owned(),
            ),
        ];
        let final_documentation_strings = concat_documentation_string(documentation_strings);
        assert_eq!(final_documentation_strings.len(), 2);
    }
}
