#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ComponentDefinition {
    pub name: String,
    pub arguments: Vec<Argument>,
    pub definition: Component,
    pub line_number: usize,
}

impl ComponentDefinition {
    pub(crate) fn new(
        name: &str,
        arguments: Vec<Argument>,
        definition: Component,
        line_number: usize,
    ) -> ComponentDefinition {
        ComponentDefinition {
            name: name.to_string(),
            arguments,
            definition,
            line_number,
        }
    }

    pub(crate) fn from_ast(
        ast: ftd::ast::AST,
        doc: &ftd::interpreter2::TDoc,
    ) -> ftd::interpreter2::Result<ComponentDefinition> {
        let component_definition = ast.get_component_definition(doc.name)?;
        let name = doc.resolve_name(component_definition.name.as_str());
        let arguments = Argument::from_ast_fields(component_definition.arguments, doc)?;
        let definition_name_with_arguments =
            (component_definition.name.as_str(), arguments.as_slice());
        let definition = Component::from_ast_component(
            component_definition.definition,
            Some(definition_name_with_arguments),
            doc,
        )?;
        Ok(ComponentDefinition::new(
            name.as_str(),
            arguments,
            definition,
            component_definition.line_number,
        ))
    }

    pub fn to_value(&self, kind: &ftd::interpreter2::KindData) -> ftd::interpreter2::Value {
        ftd::interpreter2::Value::UI {
            name: self.name.to_string(),
            kind: kind.to_owned(),
            component: self.definition.to_owned(),
        }
    }
}

pub type Argument = ftd::interpreter2::Field;

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Component {
    pub name: String,
    pub properties: Vec<Property>,
    pub iteration: Box<Option<Loop>>,
    pub condition: Box<Option<ftd::interpreter2::Boolean>>,
    pub events: Vec<Event>,
    pub children: Vec<Component>,
    pub line_number: usize,
}

impl Component {
    pub(crate) fn from_name(name: &str) -> Component {
        Component {
            name: name.to_string(),
            properties: vec![],
            iteration: Box::new(None),
            condition: Box::new(None),
            events: vec![],
            children: vec![],
            line_number: 0,
        }
    }

    pub(crate) fn is_loop(&self) -> bool {
        self.iteration.is_some()
    }

    pub(crate) fn from_ast(
        ast: ftd::ast::AST,
        doc: &ftd::interpreter2::TDoc,
    ) -> ftd::interpreter2::Result<Component> {
        let component_invocation = ast.get_component_invocation(doc.name)?;
        Component::from_ast_component(component_invocation, None, doc)
    }
    fn from_ast_component(
        ast_component: ftd::ast::Component,
        definition_name_with_arguments: Option<(&str, &[Argument])>,
        doc: &ftd::interpreter2::TDoc,
    ) -> ftd::interpreter2::Result<Component> {
        let name = doc.resolve_name(ast_component.name.as_str());

        let mut loop_object_name_and_kind = None;
        let iteration = if let Some(v) = ast_component.iteration {
            let iteration = Loop::from_ast_loop(v, definition_name_with_arguments, doc)?;
            loop_object_name_and_kind = Some((
                iteration.alias.to_string(),
                iteration.loop_object_as_argument(doc)?,
            ));
            Some(iteration)
        } else {
            None
        };

        let condition = if let Some(v) = ast_component.condition {
            Some(ftd::interpreter2::Boolean::from_ast_condition(
                v,
                definition_name_with_arguments,
                &loop_object_name_and_kind,
                doc,
            )?)
        } else {
            None
        };

        let events =
            Event::from_ast_events(ast_component.events, definition_name_with_arguments, doc)?;

        let children = {
            let mut children = vec![];
            for child in ast_component.children {
                children.push(Component::from_ast_component(
                    child,
                    definition_name_with_arguments,
                    doc,
                )?);
            }
            children
        };

        let properties = Property::from_ast_properties(
            ast_component.properties,
            ast_component.name.as_str(),
            definition_name_with_arguments,
            &loop_object_name_and_kind,
            doc,
        )?;

        Ok(Component {
            name,
            properties,
            iteration: Box::new(iteration),
            condition: Box::new(condition),
            events,
            children,
            line_number: ast_component.line_number,
        })
    }
}

pub type PropertySource = ftd::ast::PropertySource;

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Property {
    pub value: ftd::interpreter2::PropertyValue,
    pub source: ftd::interpreter2::PropertySource,
    pub condition: Option<ftd::interpreter2::Boolean>,
    pub line_number: usize,
}

impl Property {
    pub(crate) fn resolve(
        &self,
        doc: &ftd::interpreter2::TDoc,
    ) -> ftd::interpreter2::Result<Option<ftd::interpreter2::Value>> {
        Ok(match self.condition {
            Some(ref condition) if !condition.eval(doc)? => None,
            _ => Some(self.value.clone().resolve(doc, self.line_number)?),
        })
    }

    fn from_ast_properties(
        ast_properties: Vec<ftd::ast::Property>,
        component_name: &str,
        definition_name_with_arguments: Option<(&str, &[Argument])>,
        loop_object_name_and_kind: &Option<(String, ftd::interpreter2::Argument)>,
        doc: &ftd::interpreter2::TDoc,
    ) -> ftd::interpreter2::Result<Vec<Property>> {
        let mut properties = vec![];
        for property in ast_properties {
            properties.push(Property::from_ast_property(
                property,
                component_name,
                definition_name_with_arguments,
                loop_object_name_and_kind,
                doc,
            )?);
        }
        Ok(properties)
    }

    fn from_ast_property(
        ast_property: ftd::ast::Property,
        component_name: &str,
        definition_name_with_arguments: Option<(&str, &[Argument])>,
        loop_object_name_and_kind: &Option<(String, ftd::interpreter2::Argument)>,
        doc: &ftd::interpreter2::TDoc,
    ) -> ftd::interpreter2::Result<Property> {
        let argument = match definition_name_with_arguments {
            Some((name, arg)) if name.eq(component_name) => {
                Property::get_argument_for_property(&ast_property, name, arg, doc)?
            }
            _ => {
                let component = doc.get_component(component_name, ast_property.line_number)?;
                Property::get_argument_for_property(
                    &ast_property,
                    component.name.as_str(),
                    component.arguments.as_slice(),
                    doc,
                )?
            }
        };
        let value = ftd::interpreter2::PropertyValue::from_ast_value_with_argument(
            ast_property.value.to_owned(),
            doc,
            argument.mutable,
            Some(&argument.kind),
            definition_name_with_arguments,
            loop_object_name_and_kind,
        )?;

        let condition = if let Some(ref v) = ast_property.condition {
            Some(ftd::interpreter2::Boolean::from_ast_condition(
                ftd::ast::Condition::new(v, ast_property.line_number),
                definition_name_with_arguments,
                loop_object_name_and_kind,
                doc,
            )?)
        } else {
            None
        };

        if ast_property.value.is_null() && !argument.kind.is_optional() {
            return ftd::interpreter2::utils::e2(
                format!(
                    "Excepted Value for argument {} in component {}",
                    argument.name, component_name
                ),
                doc.name,
                ast_property.line_number,
            );
        }

        Ok(Property {
            value,
            source: ast_property.source,
            condition,
            line_number: ast_property.line_number,
        })
    }

    fn get_argument_for_property(
        ast_property: &ftd::ast::Property,
        component_name: &str,
        component_argument: &[Argument],
        doc: &ftd::interpreter2::TDoc,
    ) -> ftd::interpreter2::Result<Argument> {
        match &ast_property.source {
            ftd::ast::PropertySource::Caption => component_argument
                .iter()
                .find(|v| v.is_caption())
                .ok_or(ftd::interpreter2::Error::ParseError {
                    message: format!(
                        "Caption type argument not found for component `{}`",
                        component_name
                    ),
                    doc_id: doc.name.to_string(),
                    line_number: ast_property.line_number,
                })
                .map(ToOwned::to_owned),
            ftd::ast::PropertySource::Body => component_argument
                .iter()
                .find(|v| v.is_body())
                .ok_or(ftd::interpreter2::Error::ParseError {
                    message: format!(
                        "Body type argument not found for component `{}`",
                        component_name
                    ),
                    doc_id: doc.name.to_string(),
                    line_number: ast_property.line_number,
                })
                .map(ToOwned::to_owned),
            ftd::ast::PropertySource::Header { name, mutable } => {
                let argument = component_argument.iter().find(|v| v.name.eq(name)).ok_or(
                    ftd::interpreter2::Error::ParseError {
                        message: format!(
                            "Header type {} {} argument not found for component `{}`",
                            name, mutable, component_name
                        ),
                        doc_id: doc.name.to_string(),
                        line_number: ast_property.line_number,
                    },
                )?;
                if !argument.mutable.eq(mutable) {
                    let mutable = if argument.mutable {
                        "mutable"
                    } else {
                        "immutuable"
                    };
                    return ftd::interpreter2::utils::e2(
                        format!("Expected `{}` for {}", mutable, argument.name),
                        doc.name,
                        ast_property.line_number,
                    );
                }
                Ok(argument.to_owned())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Loop {
    pub on: ftd::interpreter2::PropertyValue,
    pub alias: String,
    pub line_number: usize,
}

impl Loop {
    fn new(on: ftd::interpreter2::PropertyValue, alias: &str, line_number: usize) -> Loop {
        Loop {
            on,
            alias: alias.to_string(),
            line_number,
        }
    }

    pub(crate) fn loop_object_as_argument(
        &self,
        doc: &ftd::interpreter2::TDoc,
    ) -> ftd::interpreter2::Result<ftd::interpreter2::Argument> {
        let kind_data = self.children(doc)?.1;
        Ok(ftd::interpreter2::Argument {
            name: self.alias.to_string(),
            kind: kind_data,
            mutable: false,
            value: None,
            line_number: 0,
        })
    }

    fn from_ast_loop(
        ast_loop: ftd::ast::Loop,
        definition_name_with_arguments: Option<(&str, &[Argument])>,
        doc: &ftd::interpreter2::TDoc,
    ) -> ftd::interpreter2::Result<Loop> {
        let on = ftd::interpreter2::PropertyValue::from_string_with_argument(
            ast_loop.on.as_str(),
            doc,
            None,
            false,
            ast_loop.line_number,
            definition_name_with_arguments,
            &None,
        )?;

        Ok(Loop::new(on, ast_loop.alias.as_str(), ast_loop.line_number))
    }

    pub fn children(
        &self,
        doc: &ftd::interpreter2::TDoc,
    ) -> ftd::interpreter2::Result<(
        Vec<ftd::interpreter2::PropertyValue>,
        ftd::interpreter2::KindData,
    )> {
        let value = self.on.clone().resolve(doc, self.line_number)?;
        if let ftd::interpreter2::Value::List { data, kind } = value {
            Ok((data, kind))
        } else {
            ftd::interpreter2::utils::e2(
                format!("Expected list type data, found: {:?}", self.on),
                doc.name,
                self.line_number,
            )
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Event {
    name: String,
    action: String, //TODO: to action containing ftd::interpreter2::Thing::Function
    line_number: usize,
}

impl Event {
    fn from_ast_event(
        ast_event: ftd::ast::Event,
        _definition_name_with_arguments: Option<(&str, &[Argument])>,
        _doc: &ftd::interpreter2::TDoc,
    ) -> ftd::interpreter2::Result<Event> {
        Ok(Event {
            name: ast_event.name.to_string(),
            action: ast_event.action.to_string(),
            line_number: ast_event.line_number,
        })
    }

    fn from_ast_events(
        ast_events: Vec<ftd::ast::Event>,
        definition_name_with_arguments: Option<(&str, &[Argument])>,
        doc: &ftd::interpreter2::TDoc,
    ) -> ftd::interpreter2::Result<Vec<Event>> {
        let mut events = vec![];
        for event in ast_events {
            events.push(Event::from_ast_event(
                event,
                definition_name_with_arguments,
                doc,
            )?);
        }
        Ok(events)
    }
}
