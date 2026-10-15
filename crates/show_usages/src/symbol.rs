use std::ops::Range;

use gpui::SharedString;
use language::{BufferSnapshot, Node};

const NAME_FIELDS: [&str; 3] = ["name", "pattern", "property"];
const CONSTRUCTOR_NAME: &str = "constructor";
const CONSTRUCTOR_TYPE: &str = "Constructor";
const CONST_KEYWORD: &str = "const";
const BINDING_KEYWORD_FIELD: &str = "kind";
const ASSOCIATED_ITEMS_KIND: &str = "declaration_list";
const ASSOCIATED_ITEM_OWNERS: [&str; 2] = ["impl_item", "trait_item"];
const CONTAINER_NAME_FIELDS: &[(&str, &str)] = &[
    ("class_declaration", "name"),
    ("abstract_class_declaration", "name"),
    ("class", "name"),
    ("interface_declaration", "name"),
    ("type_alias_declaration", "name"),
    ("enum_declaration", "name"),
    ("internal_module", "name"),
    ("module", "name"),
    ("function_declaration", "name"),
    ("generator_function_declaration", "name"),
    ("function_expression", "name"),
    ("generator_function", "name"),
    ("function_signature", "name"),
    ("method_definition", "name"),
    ("method_signature", "name"),
    ("abstract_method_signature", "name"),
    ("struct_item", "name"),
    ("enum_item", "name"),
    ("union_item", "name"),
    ("trait_item", "name"),
    ("mod_item", "name"),
    ("function_item", "name"),
    ("function_signature_item", "name"),
    ("impl_item", "type"),
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SymbolDescription {
    pub(crate) name: SharedString,
    pub(crate) symbol_type: Option<&'static str>,
    pub(crate) location: Option<SymbolLocation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SymbolLocation {
    pub(crate) container: Option<SharedString>,
    pub(crate) path: SharedString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SymbolTitle {
    pub(crate) emphasized: SharedString,
    pub(crate) location: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DeclarationSyntax<'a> {
    pub(crate) kind: &'a str,
    pub(crate) name: &'a str,
    pub(crate) parent_kind: Option<&'a str>,
    pub(crate) grandparent_kind: Option<&'a str>,
    pub(crate) binding_keyword: Option<&'a str>,
}

impl SymbolDescription {
    pub(crate) fn named(name: SharedString) -> Self {
        Self {
            name,
            symbol_type: None,
            location: None,
        }
    }

    pub(crate) fn title(&self) -> SymbolTitle {
        let emphasized = match self.symbol_type {
            Some(symbol_type) => SharedString::from(format!("{symbol_type} {}", self.name)),
            None => self.name.clone(),
        };
        SymbolTitle {
            emphasized,
            location: self.location.as_ref().map(SymbolLocation::text),
        }
    }
}

impl SymbolLocation {
    fn text(&self) -> SharedString {
        let text = match &self.container {
            Some(container) => format!("({container}, {})", self.path),
            None => format!("({})", self.path),
        };
        SharedString::from(text)
    }
}

impl SymbolTitle {
    pub(crate) fn text(&self) -> SharedString {
        match &self.location {
            Some(location) => SharedString::from(format!("{} {location}", self.emphasized)),
            None => self.emphasized.clone(),
        }
    }
}

impl DeclarationSyntax<'_> {
    fn is_associated_item(&self) -> bool {
        self.parent_kind == Some(ASSOCIATED_ITEMS_KIND)
            && self
                .grandparent_kind
                .is_some_and(|kind| ASSOCIATED_ITEM_OWNERS.contains(&kind))
    }
}

pub(crate) fn symbol_type(syntax: &DeclarationSyntax<'_>) -> Option<&'static str> {
    let symbol_type = match syntax.kind {
        "property_signature" => "Property",
        "public_field_definition" | "field_definition" | "field_declaration" => "Field",
        "method_definition" if syntax.name == CONSTRUCTOR_NAME => CONSTRUCTOR_TYPE,
        "method_definition" | "method_signature" | "abstract_method_signature" => "Method",
        "function_declaration" | "generator_function_declaration" => "Function",
        "function_item" | "function_signature_item" if syntax.is_associated_item() => "Method",
        "function_item" | "function_signature_item" => "Function",
        "class_declaration" | "abstract_class_declaration" | "class" => "Class",
        "interface_declaration" => "Interface",
        "type_alias_declaration" | "type_item" => "Type alias",
        "enum_declaration" | "enum_item" => "Enum",
        "enum_body" | "enum_assignment" => "Enum constant",
        "enum_variant" => "Enum variant",
        "variable_declarator" if syntax.binding_keyword == Some(CONST_KEYWORD) => "Constant",
        "variable_declarator" | "let_declaration" => "Variable",
        "required_parameter" | "optional_parameter" | "parameter" => "Parameter",
        "struct_item" => "Struct",
        "trait_item" => "Trait",
        "const_item" => "Constant",
        "static_item" => "Static",
        "mod_item" => "Module",
        "macro_definition" => "Macro",
        _ => return None,
    };
    Some(symbol_type)
}

pub(crate) fn describe_declaration(
    snapshot: &BufferSnapshot,
    name_range: Range<usize>,
    path: SharedString,
) -> SymbolDescription {
    let name_range = single_line_name_range(snapshot, name_range);
    let name = node_text(snapshot, name_range.clone());
    let declaration = declaration_node(snapshot, &name_range);
    let symbol_type =
        declaration.and_then(|declaration| symbol_type(&declaration_syntax(declaration, &name)));
    let scope = declaration.or_else(|| snapshot.syntax_ancestor(name_range));
    let container = scope.and_then(|node| container_name(snapshot, node));
    let name = match (symbol_type, &container) {
        (Some(CONSTRUCTOR_TYPE), Some(class_name)) => class_name.clone(),
        _ => SharedString::from(name),
    };
    SymbolDescription {
        name,
        symbol_type,
        location: Some(SymbolLocation { container, path }),
    }
}

pub(crate) fn describe_symbol_at(
    snapshot: &BufferSnapshot,
    offset: usize,
    path: Option<SharedString>,
) -> SymbolDescription {
    let (word_range, _) = snapshot.surrounding_word(offset, None);
    let name = SharedString::from(node_text(snapshot, word_range.clone()));
    let declared_here = !word_range.is_empty()
        && declaration_node(snapshot, &word_range).is_some_and(|declaration| {
            symbol_type(&declaration_syntax(declaration, &name)).is_some()
        });
    match path {
        Some(path) if declared_here => describe_declaration(snapshot, word_range, path),
        _ => SymbolDescription::named(name),
    }
}

fn single_line_name_range(snapshot: &BufferSnapshot, name_range: Range<usize>) -> Range<usize> {
    let spans_lines = snapshot
        .text_for_range(name_range.clone())
        .any(|chunk| chunk.contains('\n'));
    if name_range.is_empty() || spans_lines {
        snapshot.surrounding_word(name_range.start, None).0
    } else {
        name_range
    }
}

fn declaration_node<'a>(
    snapshot: &'a BufferSnapshot,
    name_range: &Range<usize>,
) -> Option<Node<'a>> {
    let enclosing = snapshot.syntax_ancestor(name_range.clone())?;
    let mut node = enclosing.named_descendant_for_byte_range(name_range.start, name_range.end)?;
    loop {
        if declares_name(node, name_range) {
            return Some(node);
        }
        if node.id() == enclosing.id() {
            return None;
        }
        node = node.parent()?;
    }
}

fn declares_name(node: Node<'_>, name_range: &Range<usize>) -> bool {
    let mut cursor = node.walk();
    NAME_FIELDS.into_iter().any(|field| {
        node.children_by_field_name(field, &mut cursor)
            .any(|child| child.byte_range() == *name_range)
    })
}

fn declaration_syntax<'a>(declaration: Node<'a>, name: &'a str) -> DeclarationSyntax<'a> {
    let parent = declaration.parent();
    DeclarationSyntax {
        kind: declaration.kind(),
        name,
        parent_kind: parent.map(|parent| parent.kind()),
        grandparent_kind: parent
            .and_then(|parent| parent.parent())
            .map(|grandparent| grandparent.kind()),
        binding_keyword: parent
            .and_then(|parent| parent.child_by_field_name(BINDING_KEYWORD_FIELD))
            .map(|keyword| keyword.kind()),
    }
}

fn container_name(snapshot: &BufferSnapshot, node: Node<'_>) -> Option<SharedString> {
    let mut ancestor = node.parent();
    while let Some(current) = ancestor {
        let name = CONTAINER_NAME_FIELDS
            .iter()
            .find(|(kind, _)| *kind == current.kind())
            .and_then(|(_, field)| current.child_by_field_name(field))
            .map(|name_node| node_text(snapshot, name_node.byte_range()))
            .filter(|name| !name.is_empty());
        if let Some(name) = name {
            return Some(SharedString::from(name));
        }
        ancestor = current.parent();
    }
    None
}

fn node_text(snapshot: &BufferSnapshot, range: Range<usize>) -> String {
    let text = snapshot.text_for_range(range).collect::<String>();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn syntax<'a>(kind: &'a str, name: &'a str) -> DeclarationSyntax<'a> {
        DeclarationSyntax {
            kind,
            name,
            parent_kind: None,
            grandparent_kind: None,
            binding_keyword: None,
        }
    }

    fn located(
        name: &str,
        symbol_type: Option<&'static str>,
        container: Option<&str>,
        path: &str,
    ) -> SymbolDescription {
        SymbolDescription {
            name: SharedString::from(name.to_string()),
            symbol_type,
            location: Some(SymbolLocation {
                container: container.map(|container| SharedString::from(container.to_string())),
                path: SharedString::from(path.to_string()),
            }),
        }
    }

    #[test]
    fn typescript_declarations_map_to_intellij_type_names() {
        assert_eq!(
            symbol_type(&syntax("property_signature", "qaseCode")),
            Some("Property")
        );
        assert_eq!(
            symbol_type(&syntax("public_field_definition", "limit")),
            Some("Field")
        );
        assert_eq!(
            symbol_type(&syntax("method_definition", "constructor")),
            Some("Constructor")
        );
        assert_eq!(
            symbol_type(&syntax("method_definition", "send")),
            Some("Method")
        );
        assert_eq!(
            symbol_type(&syntax("method_signature", "send")),
            Some("Method")
        );
        assert_eq!(
            symbol_type(&syntax("abstract_method_signature", "send")),
            Some("Method")
        );
        assert_eq!(
            symbol_type(&syntax("function_declaration", "report")),
            Some("Function")
        );
        assert_eq!(
            symbol_type(&syntax("abstract_class_declaration", "Reporter")),
            Some("Class")
        );
        assert_eq!(
            symbol_type(&syntax("interface_declaration", "QaseReporterOptions")),
            Some("Interface")
        );
        assert_eq!(
            symbol_type(&syntax("type_alias_declaration", "Code")),
            Some("Type alias")
        );
        assert_eq!(
            symbol_type(&syntax("enum_declaration", "Status")),
            Some("Enum")
        );
        assert_eq!(
            symbol_type(&syntax("enum_body", "Passed")),
            Some("Enum constant")
        );
        assert_eq!(
            symbol_type(&syntax("enum_assignment", "Failed")),
            Some("Enum constant")
        );
        assert_eq!(
            symbol_type(&syntax("optional_parameter", "code")),
            Some("Parameter")
        );
    }

    #[test]
    fn variable_declarators_are_constants_only_under_const() {
        let constant = DeclarationSyntax {
            binding_keyword: Some("const"),
            ..syntax("variable_declarator", "LIMIT")
        };
        let variable = DeclarationSyntax {
            binding_keyword: Some("let"),
            ..syntax("variable_declarator", "counter")
        };

        assert_eq!(symbol_type(&constant), Some("Constant"));
        assert_eq!(symbol_type(&variable), Some("Variable"));
        assert_eq!(
            symbol_type(&syntax("variable_declarator", "legacy")),
            Some("Variable")
        );
    }

    #[test]
    fn rust_functions_are_methods_only_inside_impl_or_trait() {
        let associated = |owner| DeclarationSyntax {
            parent_kind: Some("declaration_list"),
            grandparent_kind: Some(owner),
            ..syntax("function_item", "new")
        };

        assert_eq!(symbol_type(&associated("impl_item")), Some("Method"));
        assert_eq!(symbol_type(&associated("trait_item")), Some("Method"));
        assert_eq!(symbol_type(&associated("mod_item")), Some("Function"));
        assert_eq!(
            symbol_type(&syntax("function_item", "main")),
            Some("Function")
        );
        assert_eq!(symbol_type(&syntax("struct_item", "Point")), Some("Struct"));
        assert_eq!(
            symbol_type(&syntax("enum_variant", "Red")),
            Some("Enum variant")
        );
        assert_eq!(
            symbol_type(&syntax("field_declaration", "x")),
            Some("Field")
        );
        assert_eq!(symbol_type(&syntax("static_item", "NAME")), Some("Static"));
        assert_eq!(symbol_type(&syntax("mod_item", "inner")), Some("Module"));
        assert_eq!(
            symbol_type(&syntax("macro_definition", "noop")),
            Some("Macro")
        );
    }

    #[test]
    fn unknown_syntax_has_no_type() {
        assert_eq!(symbol_type(&syntax("call_expression", "helper")), None);
        assert_eq!(symbol_type(&syntax("member_expression", "qaseCode")), None);
    }

    #[test]
    fn title_shows_type_and_name_then_container_and_path() {
        let title = located(
            "qaseCode",
            Some("Property"),
            Some("QaseReporterOptions"),
            "automation/reporters/qase-reporter.ts",
        )
        .title();

        assert_eq!(title.emphasized, "Property qaseCode");
        assert_eq!(
            title.location.as_deref(),
            Some("(QaseReporterOptions, automation/reporters/qase-reporter.ts)")
        );
        assert_eq!(
            title.text(),
            "Property qaseCode (QaseReporterOptions, automation/reporters/qase-reporter.ts)"
        );
    }

    #[test]
    fn title_without_container_shows_only_the_path() {
        let title = located("report", Some("Function"), None, "src/report.ts").title();

        assert_eq!(title.text(), "Function report (src/report.ts)");
    }

    #[test]
    fn title_of_unknown_kind_shows_the_bare_name_with_its_location() {
        let title = located("qaseCode", None, Some("config"), "src/config.ts").title();

        assert_eq!(title.emphasized, "qaseCode");
        assert_eq!(title.text(), "qaseCode (config, src/config.ts)");
    }

    #[test]
    fn title_of_an_unresolved_symbol_is_its_name() {
        let title = SymbolDescription::named(SharedString::from("helper")).title();

        assert_eq!(title.location, None);
        assert_eq!(title.text(), "helper");
    }
}
