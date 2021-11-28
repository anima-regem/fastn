pub(crate) struct Interpreter<'a> {
    lib: &'a dyn crate::p2::Library,
    pub bag: std::collections::BTreeMap<String, crate::p2::Thing>,
    pub p1: Vec<ftd::p1::Section>,
    pub aliases: std::collections::BTreeMap<String, String>,
    pub parsed_libs: Vec<String>,
}

impl<'a> Interpreter<'a> {
    #[cfg(feature = "async")]
    pub(crate) fn interpret(
        &mut self,
        name: &str,
        s: &str,
    ) -> crate::p1::Result<Vec<ftd::Instruction>> {
        futures::executor::block_on(self.async_interpret(name, s))
    }

    // #[observed(with_result, namespace = "ftd")]
    #[cfg(feature = "async")]
    pub(crate) async fn async_interpret(
        &mut self,
        name: &str,
        s: &str,
    ) -> crate::p1::Result<Vec<ftd::Instruction>> {
        let mut d_get = std::time::Duration::new(0, 0);
        let mut d_processor = std::time::Duration::new(0, 0);
        let v = self
            .async_interpret_(name, s, true, &mut d_get, &mut d_processor)
            .await?;
        // observer::observe_string("time_get", elapsed(d_get).as_str());
        // observer::observe_string("time_processor", elapsed(d_processor).as_str());
        Ok(v)
    }

    #[cfg(not(feature = "async"))]
    pub(crate) fn interpret(
        &mut self,
        name: &str,
        s: &str,
    ) -> crate::p1::Result<Vec<ftd::Instruction>> {
        let mut d_get = std::time::Duration::new(0, 0);
        let mut d_processor = std::time::Duration::new(0, 0);
        let v = self.interpret_(name, s, true, &mut d_get, &mut d_processor)?;
        // observer::observe_string("time_get", elapsed(d_get).as_str());
        // observer::observe_string("time_processor", elapsed(d_processor).as_str());
        Ok(v)
    }

    fn library_in_the_bag(&self, name: &str) -> bool {
        self.parsed_libs.contains(&name.to_string())
    }

    fn add_library_to_bag(&mut self, name: &str) {
        if !self.library_in_the_bag(name) {
            self.parsed_libs.push(name.to_string());
        }
    }

    #[cfg(feature = "async")]
    #[async_recursion::async_recursion(?Send)]
    async fn async_interpret_(
        &mut self,
        name: &str,
        s: &str,
        is_main: bool,
        d_get: &mut std::time::Duration,
        d_processor: &mut std::time::Duration,
    ) -> crate::p1::Result<Vec<ftd::Instruction>> {
        let p1 = crate::p1::parse(s, name)?;
        let new_p1 = ftd::p2::utils::reorder(&p1, name)?;

        let mut aliases = default_aliases();
        let mut instructions: Vec<ftd::Instruction> = Default::default();

        for p1 in new_p1.iter() {
            if p1.is_commented {
                continue;
            }

            let var_data =
                ftd::variable::VariableData::get_name_kind(&p1.name, name, p1.line_number, true);
            if p1.name == "import" {
                let (library_name, alias) =
                    crate::p2::utils::parse_import(&p1.caption, name, p1.line_number)?;
                aliases.insert(alias, library_name.clone());
                let start = std::time::Instant::now();
                let s = self.lib.get_with_result(library_name.as_str()).await?;
                *d_get = d_get.saturating_add(std::time::Instant::now() - start);
                if !self.library_in_the_bag(library_name.as_str()) {
                    self.async_interpret_(
                        library_name.as_str(),
                        s.as_str(),
                        false,
                        d_get,
                        d_processor,
                    )
                    .await?;
                    self.add_library_to_bag(library_name.as_str())
                }
                continue;
            }

            // while this is a specific to entire document, we are still creating it in a loop
            // because otherwise the self.interpret() call wont compile.
            let doc = crate::p2::TDoc {
                name,
                aliases: &aliases,
                bag: &self.bag,
            };

            let mut thing = vec![];

            if p1.name.starts_with("component ") {
                // declare a function
                let d = crate::Component::from_p1(p1, &doc)?;
                thing.push((
                    doc.resolve_name(p1.line_number, &d.full_name.to_string())?,
                    crate::p2::Thing::Component(d),
                ));
                // processed_p1.push(p1.name.to_string());
            } else if p1.name.starts_with("record ") {
                // declare a record
                let d =
                    crate::p2::Record::from_p1(p1.name.as_str(), &p1.header, &doc, p1.line_number)?;
                thing.push((
                    doc.resolve_name(p1.line_number, &d.name.to_string())?,
                    crate::p2::Thing::Record(d),
                ));
            } else if p1.name.starts_with("or-type ") {
                // declare a record
                let d = crate::OrType::from_p1(p1, &doc)?;
                thing.push((
                    doc.resolve_name(p1.line_number, &d.name.to_string())?,
                    crate::p2::Thing::OrType(d),
                ));
            } else if p1.name.starts_with("map ") {
                let d = crate::Variable::map_from_p1(p1, &doc)?;
                thing.push((
                    doc.resolve_name(p1.line_number, &d.name.to_string())?,
                    crate::p2::Thing::Variable(d),
                ));
                // } else if_two_words(p1.name.as_str() {
                //   TODO: <record-name> <variable-name>: foo can be used to create a variable/
                //         Not sure if its a good idea tho.
                // }
            } else if p1.name == "container" {
                instructions.push(ftd::Instruction::ChangeContainer {
                    name: doc.resolve_name_with_instruction(
                        p1.line_number,
                        p1.caption(p1.line_number, doc.name)?.as_str(),
                        &instructions,
                    )?,
                });
            } else if let Ok(ref var_data) = var_data {
                if var_data.kind.is_some() || doc.get_thing(p1.line_number, &var_data.name).is_err()
                {
                    if var_data.is_none() || var_data.is_optional() {
                        // declare and instantiate a variable
                        let d = if p1
                            .header
                            .str(doc.name, p1.line_number, "$processor$")
                            .is_ok()
                        {
                            let name = var_data.name.to_string();
                            let start = std::time::Instant::now();
                            let value = self.lib.process(p1, &doc).await?;
                            *d_processor =
                                d_processor.saturating_add(std::time::Instant::now() - start);
                            crate::Variable {
                                name,
                                value,
                                conditions: vec![],
                            }
                        } else {
                            crate::Variable::from_p1(p1, &doc)?
                        };
                        thing.push((
                            doc.resolve_name(p1.line_number, &d.name.to_string())?,
                            crate::p2::Thing::Variable(d),
                        ));
                    } else {
                        // declare and instantiate a list
                        let d = if p1
                            .header
                            .str(doc.name, p1.line_number, "$processor$")
                            .is_ok()
                        {
                            let name = doc.resolve_name(p1.line_number, &var_data.name)?;
                            let start = std::time::Instant::now();
                            let value = self.lib.process(p1, &doc).await?;
                            *d_processor =
                                d_processor.saturating_add(std::time::Instant::now() - start);
                            crate::Variable {
                                name,
                                value,
                                conditions: vec![],
                            }
                        } else {
                            crate::Variable::list_from_p1(p1, &doc)?
                        };
                        thing.push((
                            doc.resolve_name(p1.line_number, &d.name.to_string())?,
                            crate::p2::Thing::Variable(d),
                        ));
                    }
                } else if let crate::p2::Thing::Variable(mut v) =
                    doc.get_thing(p1.line_number, var_data.name.as_str())?
                {
                    assert!(
                        !(p1.header
                            .str_optional(doc.name, p1.line_number, "if")?
                            .is_some()
                            && p1
                                .header
                                .str_optional(doc.name, p1.line_number, "$processor$")?
                                .is_some())
                    );
                    if let Some(expr) = p1.header.str_optional(doc.name, p1.line_number, "if")? {
                        let val = v.get_value(p1, &doc)?;
                        v.conditions.push((
                            crate::p2::Boolean::from_expression(
                                expr,
                                &doc,
                                &Default::default(),
                                (None, None),
                                p1.line_number,
                            )?,
                            val,
                        ));
                    } else if p1
                        .header
                        .str_optional(doc.name, p1.line_number, "$processor$")?
                        .is_some()
                    {
                        let start = std::time::Instant::now();
                        let value = self.lib.process(p1, &doc).await?;
                        *d_processor =
                            d_processor.saturating_add(std::time::Instant::now() - start);
                        v.value = value;
                    } else {
                        v.update_from_p1(p1, &doc)?;
                    }
                    thing.push((
                        doc.resolve_name(p1.line_number, &var_data.name.to_string())?,
                        crate::p2::Thing::Variable(v),
                    ));
                }
            } else {
                // cloning because https://github.com/rust-lang/rust/issues/59159
                match (doc.get_thing(p1.line_number, p1.name.as_str())?).clone() {
                    crate::p2::Thing::Variable(_) => {
                        return ftd::e2(
                            format!("variable should have prefix $, found: `{}`", p1.name),
                            doc.name,
                            p1.line_number,
                        );
                    }
                    crate::p2::Thing::Component(_) => {
                        if let Ok(loop_data) = p1.header.str(doc.name, p1.line_number, "$loop$") {
                            let section_to_subsection = ftd::p1::SubSection {
                                name: p1.name.to_string(),
                                caption: p1.caption.to_owned(),
                                header: p1.header.to_owned(),
                                body: p1.body.to_owned(),
                                is_commented: p1.is_commented,
                                line_number: p1.line_number,
                            };
                            instructions.push(ftd::Instruction::RecursiveChildComponent {
                                child: ftd::component::recursive_child_component(
                                    loop_data,
                                    &section_to_subsection,
                                    &doc,
                                    &Default::default(),
                                    None,
                                )?,
                            });
                        } else {
                            let parent = ftd::ChildComponent::from_p1(
                                p1.line_number,
                                p1.name.as_str(),
                                &p1.header,
                                &p1.caption,
                                &p1.body_without_comment(),
                                &doc,
                                &Default::default(),
                            )?;

                            let mut children = vec![];

                            for sub in p1.sub_sections.0.iter() {
                                if sub.is_commented {
                                    continue;
                                }
                                if let Ok(loop_data) =
                                    sub.header.str(doc.name, p1.line_number, "$loop$")
                                {
                                    children.push(ftd::component::recursive_child_component(
                                        loop_data,
                                        sub,
                                        &doc,
                                        &parent.arguments,
                                        None,
                                    )?);
                                } else {
                                    children.push(ftd::ChildComponent::from_p1(
                                        sub.line_number,
                                        sub.name.as_str(),
                                        &sub.header,
                                        &sub.caption,
                                        &sub.body_without_comment(),
                                        &doc,
                                        &parent.arguments,
                                    )?);
                                }
                            }

                            instructions.push(ftd::Instruction::Component { children, parent })
                        }
                    }
                    crate::p2::Thing::Record(mut r) => {
                        r.add_instance(p1, &doc)?;
                        thing.push((
                            doc.resolve_name(p1.line_number, &p1.name.to_string())?,
                            crate::p2::Thing::Record(r),
                        ));
                    }
                    crate::p2::Thing::OrType(_r) => {
                        // do we allow initialization of a record by name? nopes
                        return ftd::e2(
                            format!("'{}' is an or-type", p1.name.as_str()),
                            doc.name,
                            p1.line_number,
                        );
                    }
                    crate::p2::Thing::OrTypeWithVariant { .. } => {
                        // do we allow initialization of a record by name? nopes
                        return ftd::e2(
                            format!("'{}' is an or-type variant", p1.name.as_str(),),
                            doc.name,
                            p1.line_number,
                        );
                    }
                };
            }
            self.bag.extend(thing);
        }

        if is_main {
            self.p1 = p1;
            self.aliases = aliases;
        }
        Ok(instructions)
    }

    #[cfg(not(feature = "async"))]
    fn interpret_(
        &mut self,
        name: &str,
        s: &str,
        is_main: bool,
        d_get: &mut std::time::Duration,
        d_processor: &mut std::time::Duration,
    ) -> crate::p1::Result<Vec<ftd::Instruction>> {
        let p1 = crate::p1::parse(s, name)?;
        let new_p1 = ftd::p2::utils::reorder(&p1, name)?;

        let mut aliases = default_aliases();
        let mut instructions: Vec<ftd::Instruction> = Default::default();

        for p1 in new_p1.iter() {
            if p1.is_commented {
                continue;
            }

            let var_data =
                ftd::variable::VariableData::get_name_kind(&p1.name, name, p1.line_number, true);
            if p1.name == "import" {
                let (library_name, alias) =
                    crate::p2::utils::parse_import(&p1.caption, name, p1.line_number)?;
                aliases.insert(alias, library_name.clone());
                let start = std::time::Instant::now();
                let s = self.lib.get_with_result(library_name.as_str())?;
                *d_get = d_get.saturating_add(std::time::Instant::now() - start);
                if !self.library_in_the_bag(library_name.as_str()) {
                    self.interpret_(library_name.as_str(), s.as_str(), false, d_get, d_processor)?;
                    self.add_library_to_bag(library_name.as_str())
                }
                continue;
            }

            // while this is a specific to entire document, we are still creating it in a loop
            // because otherwise the self.interpret() call wont compile.
            let doc = crate::p2::TDoc {
                name,
                aliases: &aliases,
                bag: &self.bag,
            };

            let mut thing = vec![];

            if p1.name.starts_with("component ") {
                // declare a function
                let d = crate::Component::from_p1(p1, &doc)?;
                thing.push((
                    doc.resolve_name(p1.line_number, &d.full_name.to_string())?,
                    crate::p2::Thing::Component(d),
                ));
                // processed_p1.push(p1.name.to_string());
            } else if p1.name.starts_with("record ") {
                // declare a record
                let d =
                    crate::p2::Record::from_p1(p1.name.as_str(), &p1.header, &doc, p1.line_number)?;
                thing.push((
                    doc.resolve_name(p1.line_number, &d.name.to_string())?,
                    crate::p2::Thing::Record(d),
                ));
            } else if p1.name.starts_with("or-type ") {
                // declare a record
                let d = crate::OrType::from_p1(p1, &doc)?;
                thing.push((
                    doc.resolve_name(p1.line_number, &d.name.to_string())?,
                    crate::p2::Thing::OrType(d),
                ));
            } else if p1.name.starts_with("map ") {
                let d = crate::Variable::map_from_p1(p1, &doc)?;
                thing.push((
                    doc.resolve_name(p1.line_number, &d.name.to_string())?,
                    crate::p2::Thing::Variable(d),
                ));
                // } else if_two_words(p1.name.as_str() {
                //   TODO: <record-name> <variable-name>: foo can be used to create a variable/
                //         Not sure if its a good idea tho.
                // }
            } else if p1.name == "container" {
                instructions.push(ftd::Instruction::ChangeContainer {
                    name: doc.resolve_name_with_instruction(
                        p1.line_number,
                        p1.caption(p1.line_number, doc.name)?.as_str(),
                        &instructions,
                    )?,
                });
            } else if let Ok(ref var_data) = var_data {
                if var_data.kind.is_some() || doc.get_thing(p1.line_number, &var_data.name).is_err()
                {
                    if var_data.is_none() || var_data.is_optional() {
                        // declare and instantiate a variable
                        let d = if p1
                            .header
                            .str(doc.name, p1.line_number, "$processor$")
                            .is_ok()
                        {
                            let name = var_data.name.to_string();
                            let start = std::time::Instant::now();
                            let value = self.lib.process(p1, &doc)?;
                            *d_processor =
                                d_processor.saturating_add(std::time::Instant::now() - start);
                            crate::Variable {
                                name,
                                value,
                                conditions: vec![],
                            }
                        } else {
                            crate::Variable::from_p1(p1, &doc)?
                        };
                        thing.push((
                            doc.resolve_name(p1.line_number, &d.name.to_string())?,
                            crate::p2::Thing::Variable(d),
                        ));
                    } else {
                        // declare and instantiate a list
                        let d = if p1
                            .header
                            .str(doc.name, p1.line_number, "$processor$")
                            .is_ok()
                        {
                            let name = doc.resolve_name(p1.line_number, &var_data.name)?;
                            let start = std::time::Instant::now();
                            let value = self.lib.process(p1, &doc)?;
                            *d_processor =
                                d_processor.saturating_add(std::time::Instant::now() - start);
                            crate::Variable {
                                name,
                                value,
                                conditions: vec![],
                            }
                        } else {
                            crate::Variable::list_from_p1(p1, &doc)?
                        };
                        thing.push((
                            doc.resolve_name(p1.line_number, &d.name.to_string())?,
                            crate::p2::Thing::Variable(d),
                        ));
                    }
                } else if let crate::p2::Thing::Variable(mut v) =
                    doc.get_thing(p1.line_number, var_data.name.as_str())?
                {
                    assert!(
                        !(p1.header
                            .str_optional(doc.name, p1.line_number, "if")?
                            .is_some()
                            && p1
                                .header
                                .str_optional(doc.name, p1.line_number, "$processor$")?
                                .is_some())
                    );
                    if let Some(expr) = p1.header.str_optional(doc.name, p1.line_number, "if")? {
                        let val = v.get_value(p1, &doc)?;
                        v.conditions.push((
                            crate::p2::Boolean::from_expression(
                                expr,
                                &doc,
                                &Default::default(),
                                (None, None),
                                p1.line_number,
                            )?,
                            val,
                        ));
                    } else if p1
                        .header
                        .str_optional(doc.name, p1.line_number, "$processor$")?
                        .is_some()
                    {
                        let start = std::time::Instant::now();
                        let value = self.lib.process(p1, &doc)?;
                        *d_processor =
                            d_processor.saturating_add(std::time::Instant::now() - start);
                        v.value = value;
                    } else {
                        v.update_from_p1(p1, &doc)?;
                    }
                    thing.push((
                        doc.resolve_name(p1.line_number, &var_data.name.to_string())?,
                        crate::p2::Thing::Variable(v),
                    ));
                }
            } else {
                // cloning because https://github.com/rust-lang/rust/issues/59159
                match (doc.get_thing(p1.line_number, p1.name.as_str())?).clone() {
                    crate::p2::Thing::Variable(_) => {
                        return ftd::e2(
                            format!("variable should have prefix $, found: `{}`", p1.name),
                            doc.name,
                            p1.line_number,
                        );
                    }
                    crate::p2::Thing::Component(_) => {
                        if let Ok(loop_data) = p1.header.str(doc.name, p1.line_number, "$loop$") {
                            let section_to_subsection = ftd::p1::SubSection {
                                name: p1.name.to_string(),
                                caption: p1.caption.to_owned(),
                                header: p1.header.to_owned(),
                                body: p1.body.to_owned(),
                                is_commented: p1.is_commented,
                                line_number: p1.line_number,
                            };
                            instructions.push(ftd::Instruction::RecursiveChildComponent {
                                child: ftd::component::recursive_child_component(
                                    loop_data,
                                    &section_to_subsection,
                                    &doc,
                                    &Default::default(),
                                    None,
                                )?,
                            });
                        } else {
                            let parent = ftd::ChildComponent::from_p1(
                                p1.line_number,
                                p1.name.as_str(),
                                &p1.header,
                                &p1.caption,
                                &p1.body_without_comment(),
                                &doc,
                                &Default::default(),
                            )?;

                            let mut children = vec![];

                            for sub in p1.sub_sections.0.iter() {
                                if sub.is_commented {
                                    continue;
                                }
                                if let Ok(loop_data) =
                                    sub.header.str(doc.name, p1.line_number, "$loop$")
                                {
                                    children.push(ftd::component::recursive_child_component(
                                        loop_data,
                                        sub,
                                        &doc,
                                        &parent.arguments,
                                        None,
                                    )?);
                                } else {
                                    children.push(ftd::ChildComponent::from_p1(
                                        sub.line_number,
                                        sub.name.as_str(),
                                        &sub.header,
                                        &sub.caption,
                                        &sub.body_without_comment(),
                                        &doc,
                                        &parent.arguments,
                                    )?);
                                }
                            }

                            instructions.push(ftd::Instruction::Component { children, parent })
                        }
                    }
                    crate::p2::Thing::Record(mut r) => {
                        r.add_instance(p1, &doc)?;
                        thing.push((
                            doc.resolve_name(p1.line_number, &p1.name.to_string())?,
                            crate::p2::Thing::Record(r),
                        ));
                    }
                    crate::p2::Thing::OrType(_r) => {
                        // do we allow initialization of a record by name? nopes
                        return ftd::e2(
                            format!("'{}' is an or-type", p1.name.as_str()),
                            doc.name,
                            p1.line_number,
                        );
                    }
                    crate::p2::Thing::OrTypeWithVariant { .. } => {
                        // do we allow initialization of a record by name? nopes
                        return ftd::e2(
                            format!("'{}' is an or-type variant", p1.name.as_str(),),
                            doc.name,
                            p1.line_number,
                        );
                    }
                };
            }
            self.bag.extend(thing);
        }

        if is_main {
            self.p1 = p1;
            self.aliases = aliases;
        }
        Ok(instructions)
    }

    pub(crate) fn new(lib: &'a dyn crate::p2::Library) -> Self {
        Self {
            lib,
            bag: default_bag(),
            p1: Default::default(),
            aliases: Default::default(),
            parsed_libs: Default::default(),
        }
    }
}

pub fn interpret(
    name: &str,
    source: &str,
    lib: &dyn crate::p2::Library,
) -> crate::p1::Result<(
    std::collections::BTreeMap<String, crate::p2::Thing>,
    ftd::Column,
)> {
    let mut interpreter = Interpreter::new(lib);
    let instructions = interpreter.interpret(name, source)?;
    let mut rt = ftd::RT::from(name, interpreter.aliases, interpreter.bag, instructions);
    let main = rt.render_()?;
    Ok((rt.bag, main))
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum Thing {
    Component(ftd::Component),
    Variable(ftd::Variable),
    Record(ftd::p2::Record),
    OrType(ftd::OrType),
    OrTypeWithVariant { e: ftd::OrType, variant: String },
    // Library -> Name of library successfully parsed
}

pub fn default_bag() -> std::collections::BTreeMap<String, crate::p2::Thing> {
    std::array::IntoIter::new([
        (
            "ftd#row".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::row_function()),
        ),
        (
            "ftd#column".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::column_function()),
        ),
        (
            "ftd#text".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::text_function(false)),
        ),
        (
            "ftd#text-block".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::text_function(true)),
        ),
        (
            "ftd#code".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::code_function()),
        ),
        (
            "ftd#image".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::image_function()),
        ),
        (
            "ftd#iframe".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::iframe_function()),
        ),
        (
            "ftd#integer".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::integer_function()),
        ),
        (
            "ftd#decimal".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::decimal_function()),
        ),
        (
            "ftd#boolean".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::boolean_function()),
        ),
        (
            "ftd#scene".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::scene_function()),
        ),
        (
            "ftd#input".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::input_function()),
        ),
        (
            "ftd#null".to_string(),
            crate::p2::Thing::Component(ftd::p2::element::null()),
        ),
    ])
    .collect()
}

pub fn default_aliases() -> std::collections::BTreeMap<String, String> {
    std::array::IntoIter::new([("ftd".to_string(), "ftd".to_string())]).collect()
}

pub fn default_column() -> ftd::Column {
    ftd::Column {
        common: ftd::Common {
            width: Some(ftd::Length::Fill),
            height: Some(ftd::Length::Fill),
            position: ftd::Position::Center,
            ..Default::default()
        },
        container: ftd::Container {
            wrap: true,
            ..Default::default()
        },
    }
}

// #[cfg(test)]
// pub fn elapsed(e: std::time::Duration) -> String {
//     // NOTE: there is a copy of this function in ftd also
//     let nanos = e.subsec_nanos();
//     let fraction = match nanos {
//         t if nanos < 1000 => format!("{}ns", t),
//         t if nanos < 1_000_000 => format!("{:.*}µs", 3, f64::from(t) / 1000.0),
//         t => format!("{:.*}ms", 3, f64::from(t) / 1_000_000.0),
//     };
//     let secs = e.as_secs();
//     match secs {
//         _ if secs == 0 => fraction,
//         t if secs < 5 => format!("{}.{:06}s", t, nanos / 1000),
//         t if secs < 60 => format!("{}.{:03}s", t, nanos / 1_000_000),
//         t if secs < 3600 => format!("{}m {}s", t / 60, t % 60),
//         t if secs < 86400 => format!("{}h {}m", t / 3600, (t % 3600) / 60),
//         t => format!("{}s", t),
//     }
// }

#[cfg(test)]
mod test {
    use crate::test::*;
    use crate::{markdown_line, Instruction};

    #[test]
    fn basic() {
        let mut bag = super::default_bag();
        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.text".to_string(),
                full_name: s("foo/bar#foo"),
                properties: std::array::IntoIter::new([(
                    s("text"),
                    crate::component::Property {
                        default: Some(crate::PropertyValue::Value {
                            value: crate::Value::String {
                                text: s("hello"),
                                source: crate::TextSource::Header,
                            },
                        }),
                        conditions: vec![],
                    },
                )])
                .collect(),
                ..Default::default()
            }),
        );
        bag.insert(
            "foo/bar#x".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "x".to_string(),
                value: crate::Value::Integer { value: 10 },
                conditions: vec![],
            }),
        );

        p!(
            "
            -- component foo:
            component: ftd.text
            text: hello

            -- $x: 10
            ",
            (bag, super::default_column()),
        );
    }

    #[test]
    fn conditional_attribute() {
        let mut bag = super::default_bag();
        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                full_name: s("foo/bar#foo"),
                root: "ftd.text".to_string(),
                arguments: std::array::IntoIter::new([(s("name"), crate::p2::Kind::caption())])
                    .collect(),
                properties: std::array::IntoIter::new([
                    (
                        s("color"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::Value::String {
                                    text: "white".to_string(),
                                    source: crate::TextSource::Header,
                                },
                            }),
                            conditions: vec![
                                (
                                    crate::p2::Boolean::Equal {
                                        left: crate::PropertyValue::Reference {
                                            name: "foo/bar#present".to_string(),
                                            kind: crate::p2::Kind::boolean(),
                                        },
                                        right: crate::PropertyValue::Value {
                                            value: crate::Value::Boolean { value: true },
                                        },
                                    },
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "green".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                                (
                                    crate::p2::Boolean::Equal {
                                        left: crate::PropertyValue::Reference {
                                            name: "foo/bar#present".to_string(),
                                            kind: crate::p2::Kind::boolean(),
                                        },
                                        right: crate::PropertyValue::Value {
                                            value: crate::Value::Boolean { value: false },
                                        },
                                    },
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "red".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                            ],
                        },
                    ),
                    (
                        s("text"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Variable {
                                name: "name".to_string(),
                                kind: crate::p2::Kind::caption_or_body(),
                            }),
                            conditions: vec![],
                        },
                    ),
                ])
                .collect(),
                ..Default::default()
            }),
        );

        bag.insert(
            "foo/bar#present".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "present".to_string(),
                value: crate::Value::Boolean { value: false },
                conditions: vec![],
            }),
        );

        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello"),
            line: true,
            common: ftd::Common {
                color: Some(ftd::Color {
                    r: 255,
                    g: 0,
                    b: 0,
                    alpha: 1.0,
                }),
                conditional_attribute: std::array::IntoIter::new([(
                    s("color"),
                    ftd::ConditionalAttribute {
                        attribute_type: ftd::AttributeType::Style,
                        conditions_with_value: vec![
                            (
                                ftd::Condition {
                                    variable: s("foo/bar#present"),
                                    value: s("true"),
                                },
                                ftd::ConditionalValue {
                                    value: s("rgba(0,128,0,1)"),
                                    important: false,
                                },
                            ),
                            (
                                ftd::Condition {
                                    variable: s("foo/bar#present"),
                                    value: s("false"),
                                },
                                ftd::ConditionalValue {
                                    value: s("rgba(255,0,0,1)"),
                                    important: false,
                                },
                            ),
                        ],
                        default: Some(ftd::ConditionalValue {
                            value: s("rgba(255,255,255,1)"),
                            important: false,
                        }),
                    },
                )])
                .collect(),
                locals: std::array::IntoIter::new([(s("name@0"), s("hello"))]).collect(),
                reference: Some(s("@name@0")),
                ..Default::default()
            },
            ..Default::default()
        }));

        p!(
            "
            -- $present: false

            -- component foo:
            caption $name:
            component: ftd.text
            color: white
            color if $present: green
            color if not $present: red
            text: $name

            -- foo: hello
            ",
            (bag, main),
        );
    }

    #[test]
    fn creating_a_tree() {
        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#ft_toc".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: "foo/bar#ft_toc".to_string(),
                arguments: Default::default(),
                properties: Default::default(),
                instructions: vec![
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            events: vec![],
                            root: "foo/bar#table-of-content".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("id"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "toc_main".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            arguments: Default::default(),
                            is_recursive: false,
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "foo/bar#parent".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([
                                (
                                    s("active"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::Boolean { value: true },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("id"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "/welcome/".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "5PM Tasks".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "foo/bar#parent".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([
                                (
                                    s("id"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "/Building/".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "Log".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "foo/bar#parent".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([
                                (
                                    s("id"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "/ChildBuilding/".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "ChildLog".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChangeContainer {
                        name: "/welcome/".to_string(),
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "foo/bar#parent".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([
                                (
                                    s("id"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "/Building2/".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "Log2".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                kernel: false,
                ..Default::default()
            }),
        );

        bag.insert(
            "foo/bar#parent".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: "foo/bar#parent".to_string(),
                arguments: std::array::IntoIter::new([
                    (
                        s("active"),
                        crate::p2::Kind::Optional {
                            kind: Box::new(crate::p2::Kind::boolean()),
                        },
                    ),
                    (s("id"), crate::p2::Kind::string()),
                    (s("name"), crate::p2::Kind::caption()),
                ])
                .collect(),
                properties: std::array::IntoIter::new([
                    (
                        s("id"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Variable {
                                name: "id".to_string(),
                                kind: crate::p2::Kind::Optional {
                                    kind: Box::new(crate::p2::Kind::string()),
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                    (
                        s("open"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::variable::Value::String {
                                    text: "true".to_string(),
                                    source: crate::TextSource::Header,
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                    (
                        s("width"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::variable::Value::String {
                                    text: "fill".to_string(),
                                    source: crate::TextSource::Header,
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                ])
                .collect(),
                instructions: vec![
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: Some(ftd::p2::Boolean::IsNotNull {
                                value: ftd::PropertyValue::Variable {
                                    name: "active".to_string(),
                                    kind: crate::p2::Kind::Optional {
                                        kind: Box::new(crate::p2::Kind::boolean()),
                                    },
                                },
                            }),
                            properties: std::array::IntoIter::new([
                                (
                                    s("color"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "white".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("size"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::Integer { value: 14 },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("text"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Variable {
                                            name: "name".to_string(),
                                            kind: crate::p2::Kind::caption_or_body(),
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: Some(ftd::p2::Boolean::IsNull {
                                value: ftd::PropertyValue::Variable {
                                    name: "active".to_string(),
                                    kind: crate::p2::Kind::Optional {
                                        kind: Box::new(crate::p2::Kind::boolean()),
                                    },
                                },
                            }),
                            properties: std::array::IntoIter::new([
                                (
                                    s("color"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "#4D4D4D".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("size"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::Integer { value: 14 },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("text"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Variable {
                                            name: "name".to_string(),
                                            kind: crate::p2::Kind::caption_or_body(),
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                kernel: false,
                ..Default::default()
            }),
        );

        bag.insert(
            "foo/bar#table-of-content".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: "foo/bar#table-of-content".to_string(),
                arguments: std::array::IntoIter::new([(s("id"), crate::p2::Kind::string())])
                    .collect(),
                properties: std::array::IntoIter::new([
                    (
                        s("height"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::variable::Value::String {
                                    text: "fill".to_string(),
                                    source: crate::TextSource::Header,
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                    (
                        s("id"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Variable {
                                name: "id".to_string(),
                                kind: crate::p2::Kind::Optional {
                                    kind: Box::new(crate::p2::Kind::string()),
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                    (
                        s("width"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::variable::Value::String {
                                    text: "300".to_string(),
                                    source: crate::TextSource::Header,
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                ])
                .collect(),
                instructions: vec![],
                kernel: false,
                ..Default::default()
            }),
        );

        bag.insert(
            "foo/bar#toc-heading".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.text".to_string(),
                full_name: "foo/bar#toc-heading".to_string(),
                arguments: std::array::IntoIter::new([(s("text"), crate::p2::Kind::caption())])
                    .collect(),
                properties: std::array::IntoIter::new([
                    (
                        s("size"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::variable::Value::Integer { value: 16 },
                            }),
                            conditions: vec![],
                        },
                    ),
                    (
                        s("text"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Variable {
                                name: "text".to_string(),
                                kind: crate::p2::Kind::caption_or_body(),
                            }),
                            conditions: vec![],
                        },
                    ),
                ])
                .collect(),
                ..Default::default()
            }),
        );

        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Column(ftd::Column {
                        container: ftd::Container {
                            children: vec![ftd::Element::Column(ftd::Column {
                                container: ftd::Container {
                                    children: vec![
                                        ftd::Element::Text(ftd::Text {
                                            text: ftd::markdown_line("5PM Tasks"),
                                            line: true,
                                            common: ftd::Common {
                                                color: Some(ftd::Color {
                                                    r: 255,
                                                    g: 255,
                                                    b: 255,
                                                    alpha: 1.0,
                                                }),
                                                reference: Some(s("@name@0,0,0")),
                                                ..Default::default()
                                            },
                                            size: Some(14),
                                            ..Default::default()
                                        }),
                                        ftd::Element::Null,
                                        ftd::Element::Column(ftd::Column {
                                            container: ftd::Container {
                                                children: vec![
                                                    ftd::Element::Null,
                                                    ftd::Element::Text(ftd::Text {
                                                        text: ftd::markdown_line("Log"),
                                                        line: true,
                                                        common: ftd::Common {
                                                            color: Some(ftd::Color {
                                                                r: 77,
                                                                g: 77,
                                                                b: 77,
                                                                alpha: 1.0,
                                                            }),
                                                            reference: Some(s("@name@0,0,0,0")),
                                                            ..Default::default()
                                                        },
                                                        size: Some(14),
                                                        ..Default::default()
                                                    }),
                                                    ftd::Element::Column(ftd::Column {
                                                        container: ftd::Container {
                                                            external_children: Default::default(),
                                                            children: vec![
                                                                ftd::Element::Null,
                                                                ftd::Element::Text(ftd::Text {
                                                                    text: ftd::markdown_line(
                                                                        "ChildLog",
                                                                    ),
                                                                    line: true,
                                                                    common: ftd::Common {
                                                                        color: Some(ftd::Color {
                                                                            r: 77,
                                                                            g: 77,
                                                                            b: 77,
                                                                            alpha: 1.0,
                                                                        }),
                                                                        reference: Some(s(
                                                                            "@name@0,0,0,0,0",
                                                                        )),
                                                                        ..Default::default()
                                                                    },
                                                                    size: Some(14),
                                                                    ..Default::default()
                                                                }),
                                                            ],
                                                            open: (Some(true), None),
                                                            ..Default::default()
                                                        },
                                                        common: ftd::Common {
                                                            locals: std::array::IntoIter::new([
                                                                (
                                                                    s("id@0,0,0,0,0"),
                                                                    s("/ChildBuilding/"),
                                                                ),
                                                                (
                                                                    s("name@0,0,0,0,0"),
                                                                    s("ChildLog"),
                                                                ),
                                                            ])
                                                            .collect(),
                                                            data_id: Some(s("/ChildBuilding/")),
                                                            width: Some(ftd::Length::Fill),
                                                            ..Default::default()
                                                        },
                                                    }),
                                                ],
                                                external_children: Default::default(),
                                                open: (Some(true), None),
                                                ..Default::default()
                                            },
                                            common: ftd::Common {
                                                locals: std::array::IntoIter::new([
                                                    (s("id@0,0,0,0"), s("/Building/")),
                                                    (s("name@0,0,0,0"), s("Log")),
                                                ])
                                                .collect(),
                                                data_id: Some(s("/Building/")),
                                                width: Some(ftd::Length::Fill),
                                                ..Default::default()
                                            },
                                        }),
                                        ftd::Element::Column(ftd::Column {
                                            container: ftd::Container {
                                                external_children: Default::default(),
                                                children: vec![
                                                    ftd::Element::Null,
                                                    ftd::Element::Text(ftd::Text {
                                                        text: ftd::markdown_line("Log2"),
                                                        line: true,
                                                        common: ftd::Common {
                                                            color: Some(ftd::Color {
                                                                r: 77,
                                                                g: 77,
                                                                b: 77,
                                                                alpha: 1.0,
                                                            }),
                                                            reference: Some(s("@name@0,0,0,1")),
                                                            ..Default::default()
                                                        },
                                                        size: Some(14),
                                                        ..Default::default()
                                                    }),
                                                ],
                                                open: (Some(true), None),
                                                ..Default::default()
                                            },
                                            common: ftd::Common {
                                                locals: std::array::IntoIter::new([
                                                    (s("id@0,0,0,1"), s("/Building2/")),
                                                    (s("name@0,0,0,1"), s("Log2")),
                                                ])
                                                .collect(),
                                                data_id: Some(s("/Building2/")),
                                                width: Some(ftd::Length::Fill),
                                                ..Default::default()
                                            },
                                        }),
                                    ],
                                    external_children: Default::default(),
                                    open: (Some(true), None),
                                    ..Default::default()
                                },
                                common: ftd::Common {
                                    locals: std::array::IntoIter::new([
                                        (s("active@0,0,0"), s("true")),
                                        (s("id@0,0,0"), s("/welcome/")),
                                        (s("name@0,0,0"), s("5PM Tasks")),
                                    ])
                                    .collect(),
                                    data_id: Some(s("/welcome/")),
                                    width: Some(ftd::Length::Fill),
                                    ..Default::default()
                                },
                            })],
                            ..Default::default()
                        },
                        common: ftd::Common {
                            locals: std::array::IntoIter::new([(s("id@0,0"), s("toc_main"))])
                                .collect(),
                            data_id: Some(s("toc_main")),
                            height: Some(ftd::Length::Fill),
                            width: Some(ftd::Length::Px { value: 300 }),
                            ..Default::default()
                        },
                    })],
                    ..Default::default()
                },
                ..Default::default()
            }));

        p!(
            r"
            -- component toc-heading:
            component: ftd.text
            caption $text:
            text: $text
            size: 16


            -- component table-of-content:
            component: ftd.column
            string $id:
            id: $id
            width: 300
            height: fill


            -- component parent:
            component: ftd.column
            string $id:
            caption $name:
            optional boolean $active:
            id: $id
            width: fill
            open: true

            --- ftd.text:
            if: $active is not null
            text: $name
            size: 14
            color: white

            --- ftd.text:
            if: $active is null
            text: $name
            size: 14
            color: \#4D4D4D


            -- component ft_toc:
            component: ftd.column

            --- table-of-content:
            id: toc_main

            --- parent:
            id: /welcome/
            name: 5PM Tasks
            active: true

            --- parent:
            id: /Building/
            name: Log

            --- parent:
            id: /ChildBuilding/
            name: ChildLog

            --- container: /welcome/

            --- parent:
            id: /Building2/
            name: Log2


            -- ft_toc:
            ",
            (bag, main),
        );
    }

    #[test]
    fn creating_a_tree_using_import() {
        let mut bag = super::default_bag();

        bag.insert(
            "creating-a-tree#ft_toc".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: "creating-a-tree#ft_toc".to_string(),
                arguments: Default::default(),
                properties: Default::default(),
                instructions: vec![
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "creating-a-tree#table-of-content".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("id"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "toc_main".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "creating-a-tree#parent".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([
                                (
                                    s("active"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::Boolean { value: true },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("id"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "/welcome/".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "5PM Tasks".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "creating-a-tree#parent".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([
                                (
                                    s("id"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "/Building/".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "Log".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "creating-a-tree#parent".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([
                                (
                                    s("id"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "/ChildBuilding/".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "ChildLog".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChangeContainer {
                        name: "/welcome/".to_string(),
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "creating-a-tree#parent".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([
                                (
                                    s("id"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "/Building2/".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "Log2".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                kernel: false,
                ..Default::default()
            }),
        );

        bag.insert(
            "creating-a-tree#parent".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: "creating-a-tree#parent".to_string(),
                arguments: std::array::IntoIter::new([
                    (
                        s("active"),
                        crate::p2::Kind::Optional {
                            kind: Box::new(crate::p2::Kind::boolean()),
                        },
                    ),
                    (s("id"), crate::p2::Kind::string()),
                    (s("name"), crate::p2::Kind::caption()),
                ])
                .collect(),
                properties: std::array::IntoIter::new([
                    (
                        s("id"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Variable {
                                name: "id".to_string(),
                                kind: crate::p2::Kind::Optional {
                                    kind: Box::new(crate::p2::Kind::string()),
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                    (
                        s("open"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::variable::Value::String {
                                    text: "true".to_string(),
                                    source: crate::TextSource::Header,
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                    (
                        s("width"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::variable::Value::String {
                                    text: "fill".to_string(),
                                    source: crate::TextSource::Header,
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                ])
                .collect(),
                instructions: vec![
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: Some(ftd::p2::Boolean::IsNotNull {
                                value: ftd::PropertyValue::Variable {
                                    name: "active".to_string(),
                                    kind: crate::p2::Kind::Optional {
                                        kind: Box::new(crate::p2::Kind::boolean()),
                                    },
                                },
                            }),
                            properties: std::array::IntoIter::new([
                                (
                                    s("color"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "white".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("size"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::Integer { value: 14 },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("text"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Variable {
                                            name: "name".to_string(),
                                            kind: crate::p2::Kind::caption_or_body(),
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: Some(ftd::p2::Boolean::IsNull {
                                value: ftd::PropertyValue::Variable {
                                    name: "active".to_string(),
                                    kind: crate::p2::Kind::Optional {
                                        kind: Box::new(crate::p2::Kind::boolean()),
                                    },
                                },
                            }),
                            properties: std::array::IntoIter::new([
                                (
                                    s("color"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::String {
                                                text: "#4D4D4D".to_string(),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("size"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::variable::Value::Integer { value: 14 },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("text"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Variable {
                                            name: "name".to_string(),
                                            kind: crate::p2::Kind::caption_or_body(),
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                kernel: false,
                ..Default::default()
            }),
        );

        bag.insert(
            "creating-a-tree#table-of-content".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: "creating-a-tree#table-of-content".to_string(),
                arguments: std::array::IntoIter::new([(s("id"), crate::p2::Kind::string())])
                    .collect(),
                properties: std::array::IntoIter::new([
                    (
                        s("height"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::variable::Value::String {
                                    text: "fill".to_string(),
                                    source: crate::TextSource::Header,
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                    (
                        s("id"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Variable {
                                name: "id".to_string(),
                                kind: crate::p2::Kind::Optional {
                                    kind: Box::new(crate::p2::Kind::string()),
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                    (
                        s("width"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::variable::Value::String {
                                    text: "300".to_string(),
                                    source: crate::TextSource::Header,
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                ])
                .collect(),
                instructions: vec![],
                kernel: false,
                ..Default::default()
            }),
        );

        bag.insert(
            "creating-a-tree#toc-heading".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.text".to_string(),
                full_name: "creating-a-tree#toc-heading".to_string(),
                arguments: std::array::IntoIter::new([(s("text"), crate::p2::Kind::caption())])
                    .collect(),
                properties: std::array::IntoIter::new([
                    (
                        s("size"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::variable::Value::Integer { value: 16 },
                            }),
                            conditions: vec![],
                        },
                    ),
                    (
                        s("text"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Variable {
                                name: "text".to_string(),
                                kind: crate::p2::Kind::caption_or_body(),
                            }),
                            conditions: vec![],
                        },
                    ),
                ])
                .collect(),
                ..Default::default()
            }),
        );

        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Column(ftd::Column {
                        container: ftd::Container {
                            children: vec![ftd::Element::Column(ftd::Column {
                                container: ftd::Container {
                                    children: vec![
                                        ftd::Element::Text(ftd::Text {
                                            text: ftd::markdown_line("5PM Tasks"),
                                            line: true,
                                            common: ftd::Common {
                                                color: Some(ftd::Color {
                                                    r: 255,
                                                    g: 255,
                                                    b: 255,
                                                    alpha: 1.0,
                                                }),
                                                reference: Some(s("@name@0,0,0")),
                                                ..Default::default()
                                            },
                                            size: Some(14),
                                            ..Default::default()
                                        }),
                                        ftd::Element::Null,
                                        ftd::Element::Column(ftd::Column {
                                            container: ftd::Container {
                                                children: vec![
                                                    ftd::Element::Null,
                                                    ftd::Element::Text(ftd::Text {
                                                        text: ftd::markdown_line("Log"),
                                                        line: true,
                                                        common: ftd::Common {
                                                            color: Some(ftd::Color {
                                                                r: 77,
                                                                g: 77,
                                                                b: 77,
                                                                alpha: 1.0,
                                                            }),
                                                            reference: Some(s("@name@0,0,0,0")),
                                                            ..Default::default()
                                                        },
                                                        size: Some(14),
                                                        ..Default::default()
                                                    }),
                                                    ftd::Element::Column(ftd::Column {
                                                        container: ftd::Container {
                                                            external_children: Default::default(),
                                                            children: vec![
                                                                ftd::Element::Null,
                                                                ftd::Element::Text(ftd::Text {
                                                                    text: ftd::markdown_line(
                                                                        "ChildLog",
                                                                    ),
                                                                    line: true,
                                                                    common: ftd::Common {
                                                                        color: Some(ftd::Color {
                                                                            r: 77,
                                                                            g: 77,
                                                                            b: 77,
                                                                            alpha: 1.0,
                                                                        }),
                                                                        reference: Some(s(
                                                                            "@name@0,0,0,0,0",
                                                                        )),
                                                                        ..Default::default()
                                                                    },
                                                                    size: Some(14),
                                                                    ..Default::default()
                                                                }),
                                                            ],
                                                            open: (Some(true), None),
                                                            ..Default::default()
                                                        },
                                                        common: ftd::Common {
                                                            locals: std::array::IntoIter::new([
                                                                (
                                                                    s("id@0,0,0,0,0"),
                                                                    s("/ChildBuilding/"),
                                                                ),
                                                                (
                                                                    s("name@0,0,0,0,0"),
                                                                    s("ChildLog"),
                                                                ),
                                                            ])
                                                            .collect(),
                                                            data_id: Some(s("/ChildBuilding/")),
                                                            width: Some(ftd::Length::Fill),
                                                            ..Default::default()
                                                        },
                                                    }),
                                                ],
                                                external_children: Default::default(),
                                                open: (Some(true), None),
                                                ..Default::default()
                                            },
                                            common: ftd::Common {
                                                locals: std::array::IntoIter::new([
                                                    (s("id@0,0,0,0"), s("/Building/")),
                                                    (s("name@0,0,0,0"), s("Log")),
                                                ])
                                                .collect(),
                                                data_id: Some(s("/Building/")),
                                                width: Some(ftd::Length::Fill),
                                                ..Default::default()
                                            },
                                        }),
                                        ftd::Element::Column(ftd::Column {
                                            container: ftd::Container {
                                                external_children: Default::default(),
                                                children: vec![
                                                    ftd::Element::Null,
                                                    ftd::Element::Text(ftd::Text {
                                                        text: ftd::markdown_line("Log2"),
                                                        line: true,
                                                        common: ftd::Common {
                                                            color: Some(ftd::Color {
                                                                r: 77,
                                                                g: 77,
                                                                b: 77,
                                                                alpha: 1.0,
                                                            }),
                                                            reference: Some(s("@name@0,0,0,1")),
                                                            ..Default::default()
                                                        },
                                                        size: Some(14),
                                                        ..Default::default()
                                                    }),
                                                ],
                                                open: (Some(true), None),
                                                ..Default::default()
                                            },
                                            common: ftd::Common {
                                                locals: std::array::IntoIter::new([
                                                    (s("id@0,0,0,1"), s("/Building2/")),
                                                    (s("name@0,0,0,1"), s("Log2")),
                                                ])
                                                .collect(),
                                                data_id: Some(s("/Building2/")),
                                                width: Some(ftd::Length::Fill),
                                                ..Default::default()
                                            },
                                        }),
                                    ],
                                    external_children: Default::default(),
                                    open: (Some(true), None),
                                    ..Default::default()
                                },
                                common: ftd::Common {
                                    locals: std::array::IntoIter::new([
                                        (s("active@0,0,0"), s("true")),
                                        (s("id@0,0,0"), s("/welcome/")),
                                        (s("name@0,0,0"), s("5PM Tasks")),
                                    ])
                                    .collect(),
                                    data_id: Some(s("/welcome/")),
                                    width: Some(ftd::Length::Fill),
                                    ..Default::default()
                                },
                            })],
                            ..Default::default()
                        },
                        common: ftd::Common {
                            locals: std::array::IntoIter::new([(s("id@0,0"), s("toc_main"))])
                                .collect(),
                            data_id: Some(s("toc_main")),
                            height: Some(ftd::Length::Fill),
                            width: Some(ftd::Length::Px { value: 300 }),
                            ..Default::default()
                        },
                    })],
                    ..Default::default()
                },
                ..Default::default()
            }));

        p!(
            "
            -- import: creating-a-tree as ft

            -- ft.ft_toc:
            ",
            (bag, main),
        );
    }

    #[test]
    fn reference() {
        let mut bag = super::default_bag();

        bag.insert(
            "fifthtry/ft#dark-mode".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "dark-mode".to_string(),
                value: crate::Value::Boolean { value: true },
                conditions: vec![],
            }),
        );

        bag.insert(
            "fifthtry/ft#toc".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "toc".to_string(),
                value: crate::Value::String {
                    text: "not set".to_string(),
                    source: crate::TextSource::Caption,
                },
                conditions: vec![],
            }),
        );

        bag.insert(
            "fifthtry/ft#markdown".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.text".to_string(),
                full_name: "fifthtry/ft#markdown".to_string(),
                arguments: std::array::IntoIter::new([(s("body"), crate::p2::Kind::body())])
                    .collect(),
                properties: std::array::IntoIter::new([(
                    s("text"),
                    crate::component::Property {
                        default: Some(crate::PropertyValue::Variable {
                            name: "body".to_string(),
                            kind: crate::p2::Kind::caption_or_body(),
                        }),
                        conditions: vec![],
                    },
                )])
                .collect(),
                ..Default::default()
            }),
        );

        bag.insert(
            "reference#name".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "name".to_string(),
                value: crate::Value::String {
                    text: "John smith".to_string(),
                    source: crate::TextSource::Caption,
                },
                conditions: vec![],
            }),
        );

        bag.insert(
            "reference#test-component".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: "reference#test-component".to_string(),
                arguments: Default::default(),
                properties: std::array::IntoIter::new([
                    (
                        s("background-color"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::variable::Value::String {
                                    text: "#f3f3f3".to_string(),
                                    source: crate::TextSource::Header,
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                    (
                        s("width"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Value {
                                value: crate::variable::Value::String {
                                    text: "200".to_string(),
                                    source: crate::TextSource::Header,
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                ])
                .collect(),
                instructions: vec![crate::component::Instruction::ChildComponent {
                    child: crate::component::ChildComponent {
                        is_recursive: false,
                        events: vec![],
                        root: "ftd#text".to_string(),
                        condition: None,
                        properties: std::array::IntoIter::new([(
                            s("text"),
                            crate::component::Property {
                                default: Some(crate::PropertyValue::Reference {
                                    name: "reference#name".to_string(),
                                    kind: crate::p2::Kind::caption_or_body(),
                                }),
                                conditions: vec![],
                            },
                        )])
                        .collect(),
                        ..Default::default()
                    },
                }],
                kernel: false,
                ..Default::default()
            }),
        );
        let title = ftd::Text {
            text: ftd::markdown_line("John smith"),
            line: true,
            common: ftd::Common {
                reference: Some(s("reference#name")),
                ..Default::default()
            },
            ..Default::default()
        };

        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                common: ftd::Common {
                    width: Some(ftd::Length::Px { value: 200 }),
                    background_color: Some(ftd::Color {
                        r: 243,
                        g: 243,
                        b: 243,
                        alpha: 1.0,
                    }),
                    ..Default::default()
                },
                container: ftd::Container {
                    children: vec![ftd::Element::Text(title)],
                    ..Default::default()
                },
            }));

        p!(
            "
            -- import: reference as ct

            -- ct.test-component:
            ",
            (bag, main),
        );
    }

    #[test]
    fn text() {
        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                full_name: s("foo/bar#foo"),
                root: "ftd.text".to_string(),
                arguments: std::array::IntoIter::new([(
                    s("name"),
                    crate::p2::Kind::caption_or_body(),
                )])
                .collect(),
                properties: std::array::IntoIter::new([(
                    s("text"),
                    crate::component::Property {
                        default: Some(crate::PropertyValue::Variable {
                            name: "name".to_string(),
                            kind: crate::p2::Kind::caption_or_body(),
                        }),
                        conditions: vec![],
                    },
                )])
                .collect(),
                invocations: vec![
                    std::array::IntoIter::new([(
                        s("name"),
                        crate::Value::String {
                            text: s("hello"),
                            source: crate::TextSource::Caption,
                        },
                    )])
                    .collect(),
                    std::array::IntoIter::new([(
                        s("name"),
                        crate::Value::String {
                            text: s("world"),
                            source: crate::TextSource::Header,
                        },
                    )])
                    .collect(),
                    std::array::IntoIter::new([(
                        s("name"),
                        crate::Value::String {
                            text: s("yo yo"),
                            source: crate::TextSource::Body,
                        },
                    )])
                    .collect(),
                ],
                ..Default::default()
            }),
        );

        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello"),
            line: true,
            common: ftd::Common {
                locals: std::array::IntoIter::new([(s("name@0"), s("hello"))]).collect(),
                reference: Some(s("@name@0")),
                ..Default::default()
            },
            ..Default::default()
        }));
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("world"),
            line: true,
            common: ftd::Common {
                locals: std::array::IntoIter::new([(s("name@1"), s("world"))]).collect(),
                reference: Some(s("@name@1")),
                ..Default::default()
            },
            ..Default::default()
        }));
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown("yo yo"),
            line: false,
            common: ftd::Common {
                locals: std::array::IntoIter::new([(s("name@2"), s("yo yo"))]).collect(),
                reference: Some(s("@name@2")),
                ..Default::default()
            },
            ..Default::default()
        }));

        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component foo:
                caption or body $name:
                component: ftd.text
                text: $name

                -- foo: hello

                -- foo:
                name: world

                -- foo:

                yo yo
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_bag, bag);
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn row() {
        let mut main = super::default_column();
        let mut row = ftd::Row {
            common: ftd::Common {
                data_id: Some("the-row".to_string()),
                id: Some("the-row".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        row.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello"),
            line: true,
            ..Default::default()
        }));
        row.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("world"),
            line: true,
            ..Default::default()
        }));
        row.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("row child three"),
            line: true,
            ..Default::default()
        }));
        main.container.children.push(ftd::Element::Row(row));
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("back in main"),
            line: true,
            ..Default::default()
        }));

        p!(
            "
            -- ftd.row:
            id: the-row

            -- ftd.text:
            text: hello

            -- ftd.text:
            text: world

            -- container: ftd.main

            -- ftd.text:
            text: back in main

            -- container: the-row

            -- ftd.text:
            text: row child three
        ",
            (super::default_bag(), main),
        );
    }

    #[test]
    fn sub_function() {
        let mut main = super::default_column();
        let mut row: ftd::Row = Default::default();
        row.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello"),
            line: true,
            ..Default::default()
        }));
        row.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("world"),
            line: true,
            ..Default::default()
        }));
        main.container.children.push(ftd::Element::Row(row));
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("back in main"),
            line: true,
            ..Default::default()
        }));

        p!(
            "
            -- ftd.row:

            --- ftd.text:
            text: hello

            --- ftd.text:
            text: world

            -- ftd.text:
            text: back in main
        ",
            (super::default_bag(), main),
        );
    }

    #[test]
    fn sf1() {
        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                full_name: s("foo/bar#foo"),
                root: "ftd.row".to_string(),
                instructions: vec![crate::Instruction::ChildComponent{child: crate::ChildComponent {
                    events: vec![],
                    condition: None,
                    root: s("ftd#text"),
                    properties: std::array::IntoIter::new([
                        (
                            s("text"),
                            crate::component::Property {
                                default: Some(crate::PropertyValue::Value {
                                    value: crate::Value::String {
                                        text: s("hello"),
                                        source: crate::TextSource::Header,
                                    },
                                }),
                                conditions: vec![],
                            },
                        ),
                        (
                            s("size"),
                            crate::component::Property {
                                default: Some(crate::PropertyValue::Value {
                                    value: crate::Value::Integer { value: 14 },
                                }),
                                conditions: vec![],
                            },
                        ),
                        (
                            s("font"),
                            crate::component::Property {
                                default: Some(crate::PropertyValue::Value {
                                    value: crate::Value::String {
                                        text: s("Roboto"),
                                        source: crate::TextSource::Header,
                                    },
                                }),
                                conditions: vec![],
                            },
                        ),
                        (
                            s("font-url"),
                            crate::component::Property {
                                default: Some(crate::PropertyValue::Value {
                                    value: crate::Value::String {
                                        text: s("https://fonts.googleapis.com/css2?family=Roboto:wght@100&display=swap"),
                                        source: crate::TextSource::Header,
                                    },
                                }),
                                conditions: vec![],
                            },
                        ),
                        (
                            s("font-display"),
                            crate::component::Property {
                                default: Some(crate::PropertyValue::Value {
                                    value: crate::Value::String {
                                        text: s("swap"),
                                        source: crate::TextSource::Header,
                                    },
                                }),
                                conditions: vec![],
                            },
                        ),
                        (
                            s("border-width"),
                            crate::component::Property {
                                default: Some(crate::PropertyValue::Variable {
                                    name: s("x"),
                                    kind: crate::p2::Kind::integer().into_optional(),
                                }),
                                conditions: vec![],
                            },
                        ),
                        (
                            s("overflow-x"),
                            crate::component::Property {
                                default: Some(crate::PropertyValue::Value {
                                    value: crate::Value::String {
                                        text: s("auto"),
                                        source: crate::TextSource::Header,
                                    },
                                }),
                                conditions: vec![],
                            },
                        ),
                        (
                            s("overflow-y"),
                            crate::component::Property {
                                default: Some(crate::PropertyValue::Value {
                                    value: crate::Value::String {
                                        text: s("auto"),
                                        source: crate::TextSource::Header,
                                    },
                                }),
                                conditions: vec![],
                            },
                        ),
                    ])
                    .collect(),
                    ..Default::default()
                }}],
                arguments: std::array::IntoIter::new([(s("x"), crate::p2::Kind::integer())]).collect(),
                ..Default::default()
            }),
        );

        let mut main = super::default_column();
        let mut row: ftd::Row = Default::default();
        row.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello"),
            size: Some(14),
            external_font: Some(ftd::ExternalFont {
                url: "https://fonts.googleapis.com/css2?family=Roboto:wght@100&display=swap"
                    .to_string(),
                display: ftd::FontDisplay::Swap,
                name: "Roboto".to_string(),
            }),
            font: vec![ftd::NamedFont::Named {
                value: "Roboto".to_string(),
            }],

            line: true,
            common: ftd::Common {
                border_width: 10,
                overflow_x: Some(ftd::Overflow::Auto),
                overflow_y: Some(ftd::Overflow::Auto),
                ..Default::default()
            },
            ..Default::default()
        }));
        row.common = ftd::Common {
            locals: std::array::IntoIter::new([(s("x@0"), s("10"))]).collect(),
            ..Default::default()
        };

        main.container.children.push(ftd::Element::Row(row));
        p!(
            "
            -- component foo:
            component: ftd.row
            integer $x:

            --- ftd.text:
            text: hello
            size: 14
            border-width: $x
            font-url: https://fonts.googleapis.com/css2?family=Roboto:wght@100&display=swap
            font: Roboto
            font-display: swap
            overflow-x: auto
            overflow-y: auto

            -- foo:
            x: 10
        ",
            (bag, main),
        );
    }

    #[test]
    fn list_of_numbers() {
        let mut bag = super::default_bag();
        bag.insert(
            "foo/bar#numbers".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "foo/bar#numbers".to_string(),
                value: crate::Value::List {
                    data: vec![
                        crate::Value::Integer { value: 20 },
                        crate::Value::Integer { value: 30 },
                    ],
                    kind: crate::p2::Kind::integer(),
                },
                conditions: vec![],
            }),
        );

        p!(
            "
            -- integer list $numbers:

            -- $numbers: 20
            -- $numbers: 30
            ",
            (bag, super::default_column()),
        );
    }

    #[test]
    fn list_of_records() {
        let mut bag = super::default_bag();
        bag.insert(
            "foo/bar#point".to_string(),
            crate::p2::Thing::Record(crate::p2::Record {
                name: "foo/bar#point".to_string(),
                fields: std::array::IntoIter::new([
                    (s("x"), crate::p2::Kind::integer()),
                    (s("y"), crate::p2::Kind::integer()),
                ])
                .collect(),
                instances: Default::default(),
            }),
        );

        bag.insert(
            "foo/bar#points".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "foo/bar#points".to_string(),
                value: crate::Value::List {
                    data: vec![
                        crate::Value::Record {
                            name: s("foo/bar#point"),
                            fields: std::array::IntoIter::new([
                                (
                                    s("x"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::Integer { value: 10 },
                                    },
                                ),
                                (
                                    s("y"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::Integer { value: 20 },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                        crate::Value::Record {
                            name: s("foo/bar#point"),
                            fields: std::array::IntoIter::new([
                                (
                                    s("x"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::Integer { value: 0 },
                                    },
                                ),
                                (
                                    s("y"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::Integer { value: 0 },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                    ],
                    kind: crate::p2::Kind::Record {
                        name: s("foo/bar#point"),
                    },
                },
                conditions: vec![],
            }),
        );

        p!(
            "
            -- record point:
            integer x:
            integer y:

            -- point list $points:

            -- $points:
            x: 10
            y: 20

            -- $points:
            x: 0
            y: 0
            ",
            (bag, super::default_column()),
        );
    }

    #[test]
    #[ignore]
    fn list_with_reference() {
        let mut bag = super::default_bag();
        bag.insert(
            "foo/bar#numbers".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "foo/bar#numbers".to_string(),
                value: crate::Value::List {
                    data: vec![
                        crate::Value::Integer { value: 20 },
                        crate::Value::Integer { value: 30 },
                        // TODO: third element
                    ],
                    kind: crate::p2::Kind::integer(),
                },
                conditions: vec![],
            }),
        );
        bag.insert(
            "foo/bar#x".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "x".to_string(),
                value: crate::Value::Integer { value: 20 },
                conditions: vec![],
            }),
        );

        p!(
            "
            -- integer list $numbers:

            -- numbers: 20
            -- numbers: 30

            -- $x: 20

            -- numbers: $x
            ",
            (bag, super::default_column()),
        );
    }

    fn white_two_image_bag(
        about_optional: bool,
    ) -> std::collections::BTreeMap<String, crate::p2::Thing> {
        let mut bag = super::default_bag();
        bag.insert(
            s("foo/bar#white-two-image"),
            crate::p2::Thing::Component(crate::Component {
                invocations: Default::default(),
                full_name: "foo/bar#white-two-image".to_string(),
                root: s("ftd.column"),
                arguments: std::array::IntoIter::new([
                    (s("about"), {
                        let s = crate::p2::Kind::body();
                        if about_optional {
                            s.into_optional()
                        } else {
                            s
                        }
                    }),
                    (s("src"), {
                        let s = crate::p2::Kind::string();
                        if about_optional {
                            s.into_optional()
                        } else {
                            s
                        }
                    }),
                    (s("title"), crate::p2::Kind::caption()),
                ])
                .collect(),
                properties: std::array::IntoIter::new([(
                    s("padding"),
                    crate::component::Property {
                        default: Some(crate::PropertyValue::Value {
                            value: crate::Value::Integer { value: 30 },
                        }),
                        conditions: vec![],
                    },
                )])
                .collect(),
                kernel: false,
                instructions: vec![
                    crate::Instruction::ChildComponent {
                        child: crate::ChildComponent {
                            events: vec![],
                            condition: None,
                            root: s("ftd#text"),
                            properties: std::array::IntoIter::new([
                                (
                                    s("text"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Variable {
                                            name: s("title"),
                                            kind: crate::p2::Kind::caption_or_body(),
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("align"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::Value::String {
                                                source: crate::TextSource::Header,
                                                text: s("center"),
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::Instruction::ChildComponent {
                        child: crate::ChildComponent {
                            events: vec![],
                            condition: if about_optional {
                                Some(ftd::p2::Boolean::IsNotNull {
                                    value: crate::PropertyValue::Variable {
                                        name: s("about"),
                                        kind: crate::p2::Kind::body().into_optional(),
                                    },
                                })
                            } else {
                                None
                            },
                            root: s("ftd#text"),
                            properties: std::array::IntoIter::new([(
                                s("text"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: s("about"),
                                        kind: crate::p2::Kind::caption_or_body(),
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::Instruction::ChildComponent {
                        child: crate::ChildComponent {
                            events: vec![],
                            condition: if about_optional {
                                Some(ftd::p2::Boolean::IsNotNull {
                                    value: crate::PropertyValue::Variable {
                                        name: s("src"),
                                        kind: crate::p2::Kind::string().into_optional(),
                                    },
                                })
                            } else {
                                None
                            },
                            root: s("ftd#image"),
                            properties: std::array::IntoIter::new([(
                                s("src"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: s("src"),
                                        kind: crate::p2::Kind::string(),
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                ..Default::default()
            }),
        );
        bag
    }

    #[test]
    fn components() {
        let title = ftd::Text {
            text: ftd::markdown_line("What kind of documentation?"),
            line: true,
            common: ftd::Common {
                position: ftd::Position::Center,
                reference: Some(s("@title@0")),
                ..Default::default()
            },
            ..Default::default()
        };
        let about = ftd::Text {
            text: ftd::markdown(
                indoc::indoc!(
                    "
                    UI screens, behaviour and journeys, database tables, APIs, how to
                    contribute to, deploy, or monitor microservice, everything that
                    makes web or mobile product teams productive.
                    "
                )
                .trim(),
            ),
            common: ftd::Common {
                reference: Some(s("@about@0")),
                ..Default::default()
            },
            ..Default::default()
        };

        let image = ftd::Image {
            src: s("/static/home/document-type-min.png"),
            common: ftd::Common {
                reference: Some(s("@src@0")),
                ..Default::default()
            },
            ..Default::default()
        };

        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                common: ftd::Common {
                    padding: Some(30),
                    locals: std::array::IntoIter::new([
                        (s("about@0"), s("UI screens, behaviour and journeys, database tables, APIs, how to\ncontribute to, deploy, or monitor microservice, everything that\nmakes web or mobile product teams productive.")),
                        (
                            s("src@0"),
                            s("/static/home/document-type-min.png"),
                        ),
                        (s("title@0"), s("What kind of documentation?")),
                    ])
                    .collect(),
                    ..Default::default()
                },
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(title),
                        ftd::Element::Text(about),
                        ftd::Element::Image(image),
                    ],
                    ..Default::default()
                },
            }));

        p!(
            "
            -- component white-two-image:
            component: ftd.column
            caption $title:
            body $about:
            string $src:
            padding: 30

            --- ftd.text:
            text: $title
            align: center

            --- ftd.text:
            text: $about

            --- ftd.image:
            src: $src

            -- white-two-image: What kind of documentation?
            src: /static/home/document-type-min.png

            UI screens, behaviour and journeys, database tables, APIs, how to
            contribute to, deploy, or monitor microservice, everything that
            makes web or mobile product teams productive.
            ",
            (white_two_image_bag(false), main),
        );
    }

    #[test]
    fn conditional_body() {
        let title = ftd::Text {
            text: ftd::markdown_line("What kind of documentation?"),
            common: ftd::Common {
                position: ftd::Position::Center,
                reference: Some(s("@title@0")),
                ..Default::default()
            },
            line: true,
            ..Default::default()
        };
        let second_title = ftd::Text {
            text: ftd::markdown_line("second call"),
            common: ftd::Common {
                position: ftd::Position::Center,
                reference: Some(s("@title@1")),
                ..Default::default()
            },
            line: true,
            ..Default::default()
        };
        let about = ftd::Text {
            text: ftd::markdown(
                indoc::indoc!(
                    "
                    UI screens, behaviour and journeys, database tables, APIs, how to
                    contribute to, deploy, or monitor microservice, everything that
                    makes web or mobile product teams productive.
                    "
                )
                .trim(),
            ),
            common: ftd::Common {
                reference: Some(s("@about@0")),
                ..Default::default()
            },
            ..Default::default()
        };
        let image = ftd::Image {
            src: s("/static/home/document-type-min.png"),
            common: ftd::Common {
                reference: Some(s("@src@0")),
                ..Default::default()
            },
            ..Default::default()
        };
        let second_image = ftd::Image {
            src: s("second-image.png"),
            common: ftd::Common {
                reference: Some(s("@src@1")),
                ..Default::default()
            },
            ..Default::default()
        };

        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                common: ftd::Common {
                    padding: Some(30),
                    locals: std::array::IntoIter::new([
                        (s("about@0"), s("UI screens, behaviour and journeys, database tables, APIs, how to\ncontribute to, deploy, or monitor microservice, everything that\nmakes web or mobile product teams productive.")),
                        (
                            s("src@0"),
                            s("/static/home/document-type-min.png"),
                        ),
                        (
                            s("title@0"),
                            s("What kind of documentation?"),
                        ),
                    ])
                    .collect(),
                    ..Default::default()
                },
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(title),
                        ftd::Element::Text(about),
                        ftd::Element::Image(image),
                    ],
                    ..Default::default()
                },
            }));
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                common: ftd::Common {
                    padding: Some(30),
                    locals: std::array::IntoIter::new([
                        (s("src@1"), s("second-image.png")),
                        (s("title@1"), s("second call")),
                    ])
                    .collect(),
                    ..Default::default()
                },
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(second_title),
                        ftd::Element::Null,
                        ftd::Element::Image(second_image),
                    ],
                    ..Default::default()
                },
            }));

        p!(
            "
            -- component white-two-image:
            component: ftd.column
            caption $title:
            optional body $about:
            optional string $src:
            padding: 30

            --- ftd.text:
            text: $title
            align: center

            --- ftd.text:
            if: $about is not null
            text: $about

            --- ftd.image:
            if: $src is not null
            src: $src

            -- white-two-image: What kind of documentation?
            src: /static/home/document-type-min.png

            UI screens, behaviour and journeys, database tables, APIs, how to
            contribute to, deploy, or monitor microservice, everything that
            makes web or mobile product teams productive.

            -- white-two-image: second call
            src: second-image.png
            ",
            (white_two_image_bag(true), main),
        );
    }

    #[test]
    fn conditional_header() {
        let title = ftd::Text {
            text: ftd::markdown_line("What kind of documentation?"),
            common: ftd::Common {
                position: ftd::Position::Center,
                reference: Some(s("@title@0")),
                ..Default::default()
            },
            line: true,
            ..Default::default()
        };
        let second_title = ftd::Text {
            text: ftd::markdown_line("second call"),
            common: ftd::Common {
                position: ftd::Position::Center,
                reference: Some(s("@title@1")),
                ..Default::default()
            },
            line: true,
            ..Default::default()
        };
        let third_title = ftd::Text {
            text: ftd::markdown_line("third call"),
            common: ftd::Common {
                position: ftd::Position::Center,
                reference: Some(s("@title@2")),
                ..Default::default()
            },
            line: true,
            ..Default::default()
        };
        let about = ftd::Text {
            text: ftd::markdown(
                indoc::indoc!(
                    "
                    UI screens, behaviour and journeys, database tables, APIs, how to
                    contribute to, deploy, or monitor microservice, everything that
                    makes web or mobile product teams productive.
                    "
                )
                .trim(),
            ),
            common: ftd::Common {
                reference: Some(s("@about@0")),
                ..Default::default()
            },
            ..Default::default()
        };
        let image = ftd::Image {
            src: s("/static/home/document-type-min.png"),
            common: ftd::Common {
                reference: Some(s("@src@0")),
                ..Default::default()
            },
            ..Default::default()
        };
        let second_image = ftd::Image {
            src: s("second-image.png"),
            common: ftd::Common {
                reference: Some(s("@src@1")),
                ..Default::default()
            },
            ..Default::default()
        };

        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                common: ftd::Common {
                    padding: Some(30),
                    locals: std::array::IntoIter::new([
                        (s("about@0"), s("UI screens, behaviour and journeys, database tables, APIs, how to\ncontribute to, deploy, or monitor microservice, everything that\nmakes web or mobile product teams productive.")),
                        (
                            s("src@0"),
                            s("/static/home/document-type-min.png"),
                        ),
                        (
                            s("title@0"),
                            s("What kind of documentation?"),
                        ),
                    ])
                    .collect(),
                    ..Default::default()
                },
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(title),
                        ftd::Element::Text(about),
                        ftd::Element::Image(image),
                    ],
                    ..Default::default()
                },
            }));
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                common: ftd::Common {
                    padding: Some(30),
                    locals: std::array::IntoIter::new([
                        (s("src@1"), s("second-image.png")),
                        (s("title@1"), s("second call")),
                    ])
                    .collect(),
                    ..Default::default()
                },
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(second_title),
                        ftd::Element::Null,
                        ftd::Element::Image(second_image),
                    ],
                    ..Default::default()
                },
            }));
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                common: ftd::Common {
                    padding: Some(30),
                    locals: std::array::IntoIter::new([(s("title@2"), s("third call"))]).collect(),
                    ..Default::default()
                },
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(third_title),
                        ftd::Element::Null,
                        ftd::Element::Null,
                    ],
                    ..Default::default()
                },
            }));

        p!(
            "
            -- component white-two-image:
            component: ftd.column
            caption $title:
            optional body $about:
            optional string $src:
            padding: 30

            --- ftd.text:
            text: $title
            align: center

            --- ftd.text:
            if: $about is not null
            text: $about

            --- ftd.image:
            if: $src is not null
            src: $src

            -- white-two-image: What kind of documentation?
            src: /static/home/document-type-min.png

            UI screens, behaviour and journeys, database tables, APIs, how to
            contribute to, deploy, or monitor microservice, everything that
            makes web or mobile product teams productive.

            -- white-two-image: second call
            src: second-image.png

            -- white-two-image: third call
            ",
            (white_two_image_bag(true), main),
        );
    }

    #[test]
    fn markdown() {
        let mut bag = super::default_bag();
        bag.insert(
            s("fifthtry/ft#markdown"),
            crate::p2::Thing::Component(crate::Component {
                invocations: Default::default(),
                full_name: "fifthtry/ft#markdown".to_string(),
                root: s("ftd.text"),
                arguments: std::array::IntoIter::new([(s("body"), crate::p2::Kind::body())])
                    .collect(),
                properties: std::array::IntoIter::new([(
                    s("text"),
                    crate::component::Property {
                        default: Some(crate::PropertyValue::Variable {
                            name: s("body"),
                            kind: crate::p2::Kind::string().string_any(),
                        }),
                        conditions: vec![],
                    },
                )])
                .collect(),
                ..Default::default()
            }),
        );
        bag.insert(
            s("fifthtry/ft#dark-mode"),
            ftd::p2::Thing::Variable(ftd::Variable {
                name: s("dark-mode"),
                value: ftd::Value::Boolean { value: true },
                conditions: vec![],
            }),
        );
        bag.insert(
            s("fifthtry/ft#toc"),
            ftd::p2::Thing::Variable(ftd::Variable {
                name: s("toc"),
                value: ftd::Value::String {
                    text: "not set".to_string(),
                    source: ftd::TextSource::Caption,
                },
                conditions: vec![],
            }),
        );
        bag.insert(
            s("foo/bar#h0"),
            crate::p2::Thing::Component(crate::Component {
                invocations: Default::default(),
                full_name: "foo/bar#h0".to_string(),
                root: s("ftd.column"),
                arguments: std::array::IntoIter::new([
                    (s("body"), crate::p2::Kind::body().into_optional()),
                    (s("title"), crate::p2::Kind::caption()),
                ])
                .collect(),
                instructions: vec![
                    crate::Instruction::ChildComponent {
                        child: crate::ChildComponent {
                            events: vec![],
                            condition: None,
                            root: s("ftd#text"),
                            properties: std::array::IntoIter::new([(
                                s("text"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: s("title"),
                                        kind: crate::p2::Kind::caption_or_body(),
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::Instruction::ChildComponent {
                        child: crate::ChildComponent {
                            events: vec![],
                            condition: Some(ftd::p2::Boolean::IsNotNull {
                                value: crate::PropertyValue::Variable {
                                    name: s("body"),
                                    kind: crate::p2::Kind::body().into_optional(),
                                },
                            }),
                            root: s("fifthtry/ft#markdown"),
                            properties: std::array::IntoIter::new([(
                                s("body"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: s("body"),
                                        kind: crate::p2::Kind::body(),
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                ..Default::default()
            }),
        );

        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("hello"),
                            line: true,
                            common: ftd::Common {
                                reference: Some(s("@title@0")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown("what about the body?"),
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(
                                    s("body@0,1"),
                                    s("what about the body?"),
                                )])
                                .collect(),
                                reference: Some(s("@body@0,1")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([
                        (s("body@0"), s("what about the body?")),
                        (s("title@0"), s("hello")),
                    ])
                    .collect(),
                    ..Default::default()
                },
            }));
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("heading without body"),
                            line: true,
                            common: ftd::Common {
                                reference: Some(s("@title@1")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Null,
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([(s("title@1"), s("heading without body"))])
                        .collect(),
                    ..Default::default()
                },
            }));

        p!(
            "
            -- import: fifthtry/ft

            -- component h0:
            component: ftd.column
            caption $title:
            optional body $body:

            --- ftd.text:
            text: $title

            --- ft.markdown:
            if: $body is not null
            body: $body

            -- h0: hello

            what about the body?

            -- h0: heading without body
            ",
            (bag, main),
        );
    }

    #[test]
    fn width() {
        let mut bag = super::default_bag();

        bag.insert(
            s("foo/bar#image"),
            crate::p2::Thing::Component(crate::Component {
                invocations: Default::default(),
                full_name: "foo/bar#image".to_string(),
                root: s("ftd.column"),
                arguments: std::array::IntoIter::new([
                    (s("width"), crate::p2::Kind::string().into_optional()),
                    (s("src"), crate::p2::Kind::string()),
                ])
                .collect(),
                instructions: vec![crate::Instruction::ChildComponent {
                    child: crate::ChildComponent {
                        events: vec![],
                        condition: None,
                        root: s("ftd#image"),
                        properties: std::array::IntoIter::new([
                            (
                                s("src"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: s("src"),
                                        kind: crate::p2::Kind::string(),
                                    }),
                                    conditions: vec![],
                                },
                            ),
                            (
                                s("width"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: s("width"),
                                        kind: crate::p2::Kind::string().into_optional(),
                                    }),
                                    conditions: vec![],
                                },
                            ),
                        ])
                        .collect(),
                        ..Default::default()
                    },
                }],
                ..Default::default()
            }),
        );

        let mut main = super::default_column();

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Image(ftd::Image {
                        src: s("foo.png"),
                        common: ftd::Common {
                            reference: Some(s("@src@0")),
                            ..Default::default()
                        },
                        ..Default::default()
                    })],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([(s("src@0"), s("foo.png"))]).collect(),
                    ..Default::default()
                },
            }));
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Image(ftd::Image {
                        src: s("bar.png"),
                        common: ftd::Common {
                            reference: Some(s("@src@1")),
                            width: Some(ftd::Length::Px { value: 300 }),
                            ..Default::default()
                        },
                        ..Default::default()
                    })],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([
                        (s("src@1"), s("bar.png")),
                        (s("width@1"), s("300")),
                    ])
                    .collect(),
                    ..Default::default()
                },
            }));

        p!(
            "
            -- component image:
            component: ftd.column
            string $src:
            optional string $width:

            --- ftd.image:
            src: $src
            width: $width

            -- image:
            src: foo.png

            -- image:
            src: bar.png
            width: 300
            ",
            (bag, main),
        );
    }

    #[test]
    fn decimal() {
        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                full_name: s("foo/bar#foo"),
                root: "ftd.row".to_string(),
                instructions: vec![
                    crate::Instruction::ChildComponent {
                        child: crate::ChildComponent {
                            events: vec![],
                            condition: None,
                            root: s("ftd#decimal"),
                            properties: std::array::IntoIter::new([
                                (
                                    s("value"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::Value::Decimal { value: 0.06 },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("format"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::Value::String {
                                                text: s(".1f"),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::Instruction::ChildComponent {
                        child: crate::ChildComponent {
                            events: vec![],
                            condition: None,
                            root: s("ftd#decimal"),
                            properties: std::array::IntoIter::new([(
                                s("value"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::Value::Decimal { value: 0.01 },
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                arguments: std::array::IntoIter::new([(s("x"), crate::p2::Kind::integer())])
                    .collect(),
                ..Default::default()
            }),
        );

        let mut main = super::default_column();
        let mut row: ftd::Row = Default::default();
        row.container
            .children
            .push(ftd::Element::Decimal(ftd::Text {
                text: ftd::markdown_line("0.1"),
                line: false,
                ..Default::default()
            }));
        row.container
            .children
            .push(ftd::Element::Decimal(ftd::Text {
                text: ftd::markdown_line("0.01"),
                line: false,
                ..Default::default()
            }));
        row.common = ftd::Common {
            locals: std::array::IntoIter::new([(s("x@0"), s("10"))]).collect(),
            ..Default::default()
        };
        main.container.children.push(ftd::Element::Row(row));

        p!(
            "
            -- component foo:
            component: ftd.row
            integer $x:

            --- ftd.decimal:
            value: 0.06
            format: .1f

            --- ftd.decimal:
            value: 0.01

            -- foo:
            x: 10
        ",
            (bag, main),
        );
    }

    #[test]
    fn integer() {
        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                full_name: s("foo/bar#foo"),
                root: "ftd.row".to_string(),
                instructions: vec![
                    crate::Instruction::ChildComponent {
                        child: crate::ChildComponent {
                            events: vec![],
                            condition: None,
                            root: s("ftd#integer"),
                            properties: std::array::IntoIter::new([
                                (
                                    s("value"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::Value::Integer { value: 3 },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("format"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::Value::String {
                                                text: s("b"),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::Instruction::ChildComponent {
                        child: crate::ChildComponent {
                            events: vec![],
                            condition: None,
                            root: s("ftd#integer"),
                            properties: std::array::IntoIter::new([(
                                s("value"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::Value::Integer { value: 14 },
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                arguments: std::array::IntoIter::new([(s("x"), crate::p2::Kind::integer())])
                    .collect(),
                ..Default::default()
            }),
        );

        let mut main = super::default_column();
        let mut row: ftd::Row = Default::default();
        row.container
            .children
            .push(ftd::Element::Integer(ftd::Text {
                text: ftd::markdown_line("11"),
                line: false,
                ..Default::default()
            }));
        row.container
            .children
            .push(ftd::Element::Integer(ftd::Text {
                text: ftd::markdown_line("14"),
                line: false,
                ..Default::default()
            }));

        row.common = ftd::Common {
            locals: std::array::IntoIter::new([(s("x@0"), s("10"))]).collect(),
            ..Default::default()
        };

        main.container.children.push(ftd::Element::Row(row));

        p!(
            "
            -- component foo:
            component: ftd.row
            integer $x:

            --- ftd.integer:
            value: 3
            format: b

            --- ftd.integer:
            value: 14

            -- foo:
            x: 10
        ",
            (bag, main),
        );
    }

    #[test]
    fn boolean() {
        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                full_name: s("foo/bar#foo"),
                root: "ftd.row".to_string(),
                instructions: vec![
                    crate::Instruction::ChildComponent {
                        child: crate::ChildComponent {
                            events: vec![],
                            condition: None,
                            root: s("ftd#boolean"),
                            properties: std::array::IntoIter::new([
                                (
                                    s("value"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::Value::Boolean { value: true },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("true"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::Value::String {
                                                text: s("show this when value is true"),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("false"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::Value::String {
                                                text: s("show this when value is false"),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::Instruction::ChildComponent {
                        child: crate::ChildComponent {
                            events: vec![],
                            condition: None,
                            root: s("ftd#boolean"),
                            properties: std::array::IntoIter::new([
                                (
                                    s("value"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::Value::Boolean { value: false },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("true"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::Value::String {
                                                text: s("show this when value is true"),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("false"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Value {
                                            value: crate::Value::String {
                                                text: s("show this when value is false"),
                                                source: crate::TextSource::Header,
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                arguments: std::array::IntoIter::new([(s("x"), crate::p2::Kind::integer())])
                    .collect(),
                ..Default::default()
            }),
        );

        let mut main = super::default_column();
        let mut row: ftd::Row = Default::default();
        row.container
            .children
            .push(ftd::Element::Boolean(ftd::Text {
                text: ftd::markdown_line("show this when value is true"),
                line: false,
                ..Default::default()
            }));
        row.container
            .children
            .push(ftd::Element::Boolean(ftd::Text {
                text: ftd::markdown_line("show this when value is false"),
                line: false,
                ..Default::default()
            }));
        row.common = ftd::Common {
            locals: std::array::IntoIter::new([(s("x@0"), s("10"))]).collect(),
            ..Default::default()
        };
        main.container.children.push(ftd::Element::Row(row));

        p!(
            "
            -- component foo:
            component: ftd.row
            integer $x:

            --- ftd.boolean:
            value: true
            true:  show this when value is true
            false: show this when value is false

            --- ftd.boolean:
            value: false
            true:  show this when value is true
            false: show this when value is false

            -- foo:
            x: 10
        ",
            (bag, main),
        );
    }

    #[test]
    fn boolean_expression() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("present is true"),
            line: true,
            common: ftd::Common {
                condition: Some(ftd::Condition {
                    variable: "foo/bar#present".to_string(),
                    value: "true".to_string(),
                }),
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("present is false"),
            line: true,
            common: ftd::Common {
                condition: Some(ftd::Condition {
                    variable: "foo/bar#present".to_string(),
                    value: "false".to_string(),
                }),
                is_not_visible: true,
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("dark-mode is true"),
            line: true,
            common: ftd::Common {
                condition: Some(ftd::Condition {
                    variable: "fifthtry/ft#dark-mode".to_string(),
                    value: "true".to_string(),
                }),
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("dark-mode is false"),
            line: true,
            common: ftd::Common {
                condition: Some(ftd::Condition {
                    variable: "fifthtry/ft#dark-mode".to_string(),
                    value: "false".to_string(),
                }),
                is_not_visible: true,
                ..Default::default()
            },
            ..Default::default()
        }));

        let mut column: ftd::Column = Default::default();
        column
            .container
            .children
            .push(ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("inner present false"),
                line: true,
                common: ftd::Common {
                    condition: Some(ftd::Condition {
                        variable: "foo/bar#present".to_string(),
                        value: "false".to_string(),
                    }),
                    is_not_visible: true,
                    ..Default::default()
                },
                ..Default::default()
            }));

        column
            .container
            .children
            .push(ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("inner present true"),
                line: true,
                common: ftd::Common {
                    condition: Some(ftd::Condition {
                        variable: "foo/bar#present".to_string(),
                        value: "true".to_string(),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }));

        main.container.children.push(ftd::Element::Column(column));

        let mut column: ftd::Column = Default::default();
        column
            .container
            .children
            .push(ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("argument present false"),
                line: true,
                common: ftd::Common {
                    condition: Some(ftd::Condition {
                        variable: s("@present@5"),
                        value: s("false"),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }));
        column
            .container
            .children
            .push(ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("argument present true"),
                line: true,
                common: ftd::Common {
                    condition: Some(ftd::Condition {
                        variable: s("@present@5"),
                        value: s("true"),
                    }),
                    is_not_visible: true,
                    ..Default::default()
                },
                ..Default::default()
            }));

        column.common = ftd::Common {
            locals: std::array::IntoIter::new([(s("present@5"), s("false"))]).collect(),
            ..Default::default()
        };

        main.container.children.push(ftd::Element::Column(column));

        let mut column: ftd::Column = Default::default();
        column
            .container
            .children
            .push(ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("argument present false"),
                line: true,
                common: ftd::Common {
                    condition: Some(ftd::Condition {
                        variable: s("@present@6"),
                        value: s("false"),
                    }),
                    is_not_visible: true,
                    ..Default::default()
                },
                ..Default::default()
            }));
        column
            .container
            .children
            .push(ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("argument present true"),
                line: true,
                common: ftd::Common {
                    condition: Some(ftd::Condition {
                        variable: s("@present@6"),
                        value: s("true"),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }));

        column.common = ftd::Common {
            locals: std::array::IntoIter::new([(s("present@6"), s("true"))]).collect(),
            ..Default::default()
        };

        main.container.children.push(ftd::Element::Column(column));

        let mut column: ftd::Column = Default::default();
        column
            .container
            .children
            .push(ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("foo2 dark-mode is true"),
                line: true,
                common: ftd::Common {
                    condition: Some(ftd::Condition {
                        variable: "fifthtry/ft#dark-mode".to_string(),
                        value: "true".to_string(),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }));

        column
            .container
            .children
            .push(ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("foo2 dark-mode is false"),
                line: true,
                common: ftd::Common {
                    condition: Some(ftd::Condition {
                        variable: "fifthtry/ft#dark-mode".to_string(),
                        value: "false".to_string(),
                    }),
                    is_not_visible: true,
                    ..Default::default()
                },
                ..Default::default()
            }));

        main.container.children.push(ftd::Element::Column(column));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello literal truth"),
            line: true,
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Null);

        p!(
            "
            -- import: fifthtry/ft
            -- $present: true

            -- ftd.text: present is true
            if: $present

            -- ftd.text: present is false
            if: not $present

            -- ftd.text: dark-mode is true
            if: $ft.dark-mode

            -- ftd.text: dark-mode is false
            if: not $ft.dark-mode

            -- component foo:
            component: ftd.column

            --- ftd.text: inner present false
            if: not $present

            --- ftd.text: inner present true
            if: $present

            -- foo:

            -- component bar:
            component: ftd.column
            boolean $present:

            --- ftd.text: argument present false
            if: not $present

            --- ftd.text: argument present true
            if: $present

            -- bar:
            present: false

            -- bar:
            present: $ft.dark-mode

            -- component foo2:
            component: ftd.column

            --- ftd.text: foo2 dark-mode is true
            if: $ft.dark-mode

            --- ftd.text: foo2 dark-mode is false
            if: not $ft.dark-mode

            -- foo2:

            -- ftd.text: hello literal truth
            if: true

            -- ftd.text: never see light of the day
            if: false
        ",
            (Default::default(), main),
        );
    }

    #[test]
    fn inner_container() {
        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: "foo/bar#foo".to_string(),
                instructions: vec![
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#row".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("id"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "r1".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#row".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("id"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "r2".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                ..Default::default()
            }),
        );
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Row(ftd::Row {
                        container: ftd::Container {
                            children: vec![
                                ftd::Element::Row(ftd::Row {
                                    common: ftd::Common {
                                        data_id: Some(s("r2")),
                                        id: Some(s("foo-1:r2")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }),
                                ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("hello"),
                                    line: true,
                                    ..Default::default()
                                }),
                            ],
                            ..Default::default()
                        },
                        common: ftd::Common {
                            data_id: Some(s("r1")),
                            id: Some(s("foo-1:r1")),
                            ..Default::default()
                        },
                    })],
                    ..Default::default()
                },
                common: ftd::Common {
                    data_id: Some(s("foo-1")),
                    id: Some(s("foo-1")),
                    ..Default::default()
                },
            }));

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Row(ftd::Row {
                        container: ftd::Container {
                            children: vec![ftd::Element::Row(ftd::Row {
                                common: ftd::Common {
                                    data_id: Some(s("r2")),
                                    id: Some(s("foo-2:r2")),
                                    ..Default::default()
                                },
                                ..Default::default()
                            })],
                            ..Default::default()
                        },
                        common: ftd::Common {
                            data_id: Some(s("r1")),
                            id: Some(s("foo-2:r1")),
                            ..Default::default()
                        },
                    })],
                    ..Default::default()
                },
                common: ftd::Common {
                    data_id: Some(s("foo-2")),
                    id: Some(s("foo-2")),
                    ..Default::default()
                },
            }));

        p!(
            "
            -- component foo:
            component: ftd.column

            --- ftd.row:
            id: r1

            --- ftd.row:
            id: r2

            -- foo:
            id: foo-1

            -- foo:
            id: foo-2

            -- container: foo-1.r1

            -- ftd.text: hello
            ",
            (bag, main),
        );
    }

    #[test]
    fn inner_container_using_import() {
        let mut bag = super::default_bag();

        bag.insert(
            "inner_container#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: "inner_container#foo".to_string(),
                instructions: vec![
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#row".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("id"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "r1".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#row".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("id"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "r2".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                ..Default::default()
            }),
        );
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Row(ftd::Row {
                        container: ftd::Container {
                            children: vec![
                                ftd::Element::Row(ftd::Row {
                                    common: ftd::Common {
                                        data_id: Some(s("r2")),
                                        id: Some(s("foo-1:r2")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }),
                                ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("hello"),
                                    line: true,
                                    ..Default::default()
                                }),
                            ],
                            ..Default::default()
                        },
                        common: ftd::Common {
                            data_id: Some(s("r1")),
                            id: Some(s("foo-1:r1")),
                            ..Default::default()
                        },
                    })],
                    ..Default::default()
                },
                common: ftd::Common {
                    data_id: Some(s("foo-1")),
                    id: Some(s("foo-1")),
                    ..Default::default()
                },
            }));

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Row(ftd::Row {
                        container: ftd::Container {
                            children: vec![ftd::Element::Row(ftd::Row {
                                common: ftd::Common {
                                    data_id: Some(s("r2")),
                                    id: Some(s("foo-2:r2")),
                                    ..Default::default()
                                },
                                ..Default::default()
                            })],
                            ..Default::default()
                        },
                        common: ftd::Common {
                            data_id: Some(s("r1")),
                            id: Some(s("foo-2:r1")),
                            ..Default::default()
                        },
                    })],
                    ..Default::default()
                },
                common: ftd::Common {
                    data_id: Some(s("foo-2")),
                    id: Some(s("foo-2")),
                    ..Default::default()
                },
            }));

        p!(
            "
            -- import: inner_container as ic

            -- ic.foo:
            id: foo-1

            -- ic.foo:
            id: foo-2

            -- container: foo-1.r1

            -- ftd.text: hello
            ",
            (bag, main),
        );
    }

    #[test]
    fn open_container_with_id() {
        let mut external_children = super::default_column();
        external_children.container.children = vec![ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello"),
            line: true,
            ..Default::default()
        })];

        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    external_children: Some((
                        s("some-child"),
                        vec![vec![0, 0]],
                        vec![ftd::Element::Column(external_children)],
                    )),
                    children: vec![ftd::Element::Row(ftd::Row {
                        container: ftd::Container {
                            children: vec![ftd::Element::Row(ftd::Row {
                                common: ftd::Common {
                                    data_id: Some(s("some-child")),
                                    ..Default::default()
                                },
                                ..Default::default()
                            })],
                            ..Default::default()
                        },
                        ..Default::default()
                    })],
                    open: (None, Some(s("some-child"))),
                    ..Default::default()
                },
                ..Default::default()
            }));

        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: s("foo/bar#foo"),
                properties: std::array::IntoIter::new([(
                    s("open"),
                    crate::component::Property {
                        default: Some(crate::PropertyValue::Value {
                            value: crate::Value::String {
                                text: s("some-child"),
                                source: crate::TextSource::Header,
                            },
                        }),
                        conditions: vec![],
                    },
                )])
                .collect(),
                instructions: vec![
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            events: vec![],
                            root: "ftd#row".to_string(),
                            condition: None,
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#row".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("id"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "some-child".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                ..Default::default()
            }),
        );

        p!(
            "
            -- component foo:
            open: some-child
            component: ftd.column

            --- ftd.row:

            --- ftd.row:
            id: some-child

            -- foo:

            -- ftd.text: hello
            ",
            (bag, main),
        );
    }

    #[test]
    fn open_container_with_if() {
        let mut external_children = super::default_column();
        external_children.container.children = vec![
            ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("hello"),
                line: true,
                ..Default::default()
            }),
            ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("hello1"),
                line: true,
                ..Default::default()
            }),
        ];

        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("Start Browser"),
            line: true,
            ..Default::default()
        }));

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Column(ftd::Column {
                        container: ftd::Container {
                            children: vec![ftd::Element::Column(ftd::Column {
                                container: ftd::Container {
                                    children: vec![
                                        ftd::Element::Column(ftd::Column {
                                            container: ftd::Container {
                                                children: vec![ftd::Element::Text(ftd::Text {
                                                    text: ftd::markdown_line("Mobile Display"),
                                                    line: true,
                                                    common: ftd::Common {
                                                        data_id: Some(s("mobile-display")),
                                                        id: Some(s(
                                                            "foo-id:some-child:mobile-display",
                                                        )),
                                                        ..Default::default()
                                                    },
                                                    ..Default::default()
                                                })],
                                                ..Default::default()
                                            },
                                            common: ftd::Common {
                                                locals: std::array::IntoIter::new([(
                                                    s("id@1,0,0,0"),
                                                    s("some-child"),
                                                )])
                                                .collect(),
                                                condition: Some(ftd::Condition {
                                                    variable: s("foo/bar#mobile"),
                                                    value: s("true"),
                                                }),
                                                data_id: Some(s("some-child")),
                                                id: Some(s("foo-id:some-child")),
                                                ..Default::default()
                                            },
                                        }),
                                        ftd::Element::Column(ftd::Column {
                                            container: ftd::Container {
                                                children: vec![ftd::Element::Text(ftd::Text {
                                                    text: ftd::markdown_line("Desktop Display"),
                                                    line: true,
                                                    ..Default::default()
                                                })],
                                                ..Default::default()
                                            },
                                            common: ftd::Common {
                                                locals: std::array::IntoIter::new([(
                                                    s("id@1,0,0,1"),
                                                    s("some-child"),
                                                )])
                                                .collect(),
                                                condition: Some(ftd::Condition {
                                                    variable: s("foo/bar#mobile"),
                                                    value: s("false"),
                                                }),
                                                is_not_visible: true,
                                                data_id: Some(s("some-child")),
                                                id: Some(s("foo-id:some-child")),
                                                ..Default::default()
                                            },
                                        }),
                                    ],
                                    external_children: Some((
                                        s("some-child"),
                                        vec![vec![0], vec![1]],
                                        vec![ftd::Element::Column(external_children)],
                                    )),
                                    open: (None, Some(s("some-child"))),
                                    ..Default::default()
                                },
                                common: ftd::Common {
                                    locals: std::array::IntoIter::new([(
                                        s("id@1,0,0"),
                                        s("foo-id"),
                                    )])
                                    .collect(),
                                    id: Some(s("foo-id")),
                                    data_id: Some(s("foo-id")),
                                    ..Default::default()
                                },
                            })],
                            ..Default::default()
                        },
                        common: ftd::Common {
                            data_id: Some(s("c2")),
                            id: Some(s("c2")),
                            ..Default::default()
                        },
                    })],
                    ..Default::default()
                },
                common: ftd::Common {
                    data_id: Some(s("c1")),
                    id: Some(s("c1")),
                    ..Default::default()
                },
            }));

        let mut bag = super::default_bag();
        bag.insert(
            s("foo/bar#desktop-display"),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: s("foo/bar#desktop-display"),
                arguments: std::array::IntoIter::new([(
                    s("id"),
                    crate::p2::Kind::optional(ftd::p2::Kind::string()),
                )])
                .collect(),
                properties: std::array::IntoIter::new([(
                    s("id"),
                    ftd::component::Property {
                        default: Some(crate::PropertyValue::Variable {
                            name: "id".to_string(),
                            kind: crate::p2::Kind::Optional {
                                kind: Box::new(crate::p2::Kind::string()),
                            },
                        }),
                        conditions: vec![],
                    },
                )])
                .collect(),
                instructions: vec![crate::component::Instruction::ChildComponent {
                    child: crate::component::ChildComponent {
                        is_recursive: false,
                        events: vec![],
                        root: "ftd#text".to_string(),
                        condition: None,
                        properties: std::array::IntoIter::new([(
                            s("text"),
                            crate::component::Property {
                                default: Some(crate::PropertyValue::Value {
                                    value: crate::variable::Value::String {
                                        text: s("Desktop Display"),
                                        source: ftd::TextSource::Caption,
                                    },
                                }),
                                conditions: vec![],
                            },
                        )])
                        .collect(),
                        ..Default::default()
                    },
                }],
                ..Default::default()
            }),
        );

        bag.insert(
            s("foo/bar#foo"),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: s("foo/bar#foo"),
                properties: std::array::IntoIter::new([(
                    s("open"),
                    ftd::component::Property {
                        default: Some(crate::PropertyValue::Value {
                            value: crate::variable::Value::String {
                                text: s("some-child"),
                                source: ftd::TextSource::Header,
                            },
                        }),
                        conditions: vec![],
                    },
                )])
                .collect(),
                instructions: vec![
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "foo/bar#mobile-display".to_string(),
                            condition: Some(ftd::p2::Boolean::Equal {
                                left: ftd::PropertyValue::Reference {
                                    name: s("foo/bar#mobile"),
                                    kind: ftd::p2::Kind::Boolean { default: None },
                                },
                                right: ftd::PropertyValue::Value {
                                    value: ftd::variable::Value::Boolean { value: true },
                                },
                            }),
                            properties: std::array::IntoIter::new([(
                                s("id"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: s("some-child"),
                                            source: ftd::TextSource::Header,
                                        },
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "foo/bar#desktop-display".to_string(),
                            condition: Some(ftd::p2::Boolean::Equal {
                                left: ftd::PropertyValue::Reference {
                                    name: s("foo/bar#mobile"),
                                    kind: ftd::p2::Kind::Boolean { default: None },
                                },
                                right: ftd::PropertyValue::Value {
                                    value: ftd::variable::Value::Boolean { value: false },
                                },
                            }),
                            properties: std::array::IntoIter::new([(
                                s("id"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: s("some-child"),
                                            source: ftd::TextSource::Header,
                                        },
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                ..Default::default()
            }),
        );

        bag.insert(
            s("foo/bar#mobile"),
            crate::p2::Thing::Variable(ftd::Variable {
                name: s("mobile"),
                value: ftd::variable::Value::Boolean { value: true },
                conditions: vec![],
            }),
        );

        bag.insert(
            s("foo/bar#mobile-display"),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.column".to_string(),
                full_name: s("foo/bar#mobile-display"),
                arguments: std::array::IntoIter::new([(
                    s("id"),
                    crate::p2::Kind::optional(ftd::p2::Kind::string()),
                )])
                .collect(),
                properties: std::array::IntoIter::new([(
                    s("id"),
                    ftd::component::Property {
                        default: Some(crate::PropertyValue::Variable {
                            name: "id".to_string(),
                            kind: crate::p2::Kind::Optional {
                                kind: Box::new(crate::p2::Kind::string()),
                            },
                        }),
                        conditions: vec![],
                    },
                )])
                .collect(),
                instructions: vec![crate::component::Instruction::ChildComponent {
                    child: crate::component::ChildComponent {
                        is_recursive: false,
                        events: vec![],
                        root: "ftd#text".to_string(),
                        condition: None,
                        properties: std::array::IntoIter::new([
                            (
                                s("id"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: s("mobile-display"),
                                            source: ftd::TextSource::Header,
                                        },
                                    }),
                                    conditions: vec![],
                                },
                            ),
                            (
                                s("text"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: s("Mobile Display"),
                                            source: ftd::TextSource::Caption,
                                        },
                                    }),
                                    conditions: vec![],
                                },
                            ),
                        ])
                        .collect(),
                        ..Default::default()
                    },
                }],
                ..Default::default()
            }),
        );

        p!(
            "
            -- component mobile-display:
            component: ftd.column
            optional string $id:
            id: $id

            --- ftd.text: Mobile Display
            id: mobile-display

            -- component desktop-display:
            component: ftd.column
            optional string $id:
            id: $id

            --- ftd.text: Desktop Display

            -- $mobile: true

            -- component foo:
            open: some-child
            component: ftd.column

            --- mobile-display:
            if: $mobile
            id: some-child

            --- desktop-display:
            if: not $mobile
            id: some-child

            -- ftd.text: Start Browser

            -- ftd.column:
            id: c1

            -- ftd.column:
            id: c2

            -- foo:
            id: foo-id

            -- ftd.text: hello

            -- ftd.text: hello1
            ",
            (bag, main),
        );
    }

    #[test]
    fn nested_open_container() {
        let mut external_children = super::default_column();
        external_children.container.children = vec![
            ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("hello"),
                line: true,
                ..Default::default()
            }),
            ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("hello again"),
                line: true,
                ..Default::default()
            }),
        ];

        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Column(ftd::Column {
                        container: ftd::Container {
                            children: vec![
                                ftd::Element::Column(ftd::Column {
                                    container: ftd::Container {
                                        children: vec![ftd::Element::Column(ftd::Column {
                                            container: ftd::Container {
                                                children: vec![],
                                                ..Default::default()
                                            },
                                            common: ftd::Common {
                                                data_id: Some(s("desktop-container")),
                                                ..Default::default()
                                            },
                                        })],
                                        external_children: Some((
                                            s("desktop-container"),
                                            vec![vec![0]],
                                            vec![],
                                        )),
                                        open: (None, Some(s("desktop-container"))),
                                        ..Default::default()
                                    },
                                    common: ftd::Common {
                                        condition: Some(ftd::Condition {
                                            variable: s("foo/bar#is-mobile"),
                                            value: s("false"),
                                        }),
                                        is_not_visible: true,
                                        data_id: Some(s("main-container")),
                                        ..Default::default()
                                    },
                                }),
                                ftd::Element::Column(ftd::Column {
                                    container: ftd::Container {
                                        children: vec![ftd::Element::Column(ftd::Column {
                                            common: ftd::Common {
                                                data_id: Some(s("mobile-container")),
                                                ..Default::default()
                                            },
                                            ..Default::default()
                                        })],
                                        external_children: Some((
                                            s("mobile-container"),
                                            vec![vec![0]],
                                            vec![],
                                        )),
                                        open: (None, Some(s("mobile-container"))),
                                        ..Default::default()
                                    },
                                    common: ftd::Common {
                                        condition: Some(ftd::Condition {
                                            variable: s("foo/bar#is-mobile"),
                                            value: s("true"),
                                        }),
                                        data_id: Some(s("main-container")),
                                        ..Default::default()
                                    },
                                }),
                            ],
                            ..Default::default()
                        },
                        common: ftd::Common {
                            data_id: Some(s("start")),
                            ..Default::default()
                        },
                    })],
                    external_children: Some((
                        s("main-container"),
                        vec![vec![0, 0], vec![0, 1]],
                        vec![ftd::Element::Column(external_children)],
                    )),
                    open: (None, Some(s("main-container"))),
                    ..Default::default()
                },
                ..Default::default()
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component desktop:
                component: ftd.column
                open: desktop-container

                --- ftd.column:
                id: desktop-container

                -- component mobile:
                component: ftd.column
                open: mobile-container

                --- ftd.column:
                id: mobile-container

                -- $is-mobile: true

                -- component page:
                component: ftd.column
                open: main-container

                --- ftd.column:
                id: start

                --- desktop:
                if: not $is-mobile
                id: main-container

                --- container: start

                --- mobile:
                if: $is-mobile
                id: main-container

                -- page:

                -- ftd.text: hello

                -- ftd.text: hello again
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn deep_open_container_call() {
        let mut external_children = super::default_column();
        external_children.container.children = vec![
            ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("hello"),
                line: true,
                ..Default::default()
            }),
            ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("hello again"),
                line: true,
                ..Default::default()
            }),
        ];

        let mut main = super::default_column();

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Column(ftd::Column {
                                    common: ftd::Common {
                                        data_id: Some(s("foo")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(
                                    s("id@0,0"),
                                    s("main-container"),
                                )])
                                .collect(),
                                condition: Some(ftd::Condition {
                                    variable: s("foo/bar#is-mobile"),
                                    value: s("false"),
                                }),
                                is_not_visible: true,
                                data_id: Some(s("main-container")),
                                ..Default::default()
                            },
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Column(ftd::Column {
                                    common: ftd::Common {
                                        data_id: Some(s("foo")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(
                                    s("id@0,1"),
                                    s("main-container"),
                                )])
                                .collect(),
                                condition: Some(ftd::Condition {
                                    variable: s("foo/bar#is-mobile"),
                                    value: s("true"),
                                }),
                                data_id: Some(s("main-container")),
                                ..Default::default()
                            },
                        }),
                    ],
                    external_children: Some((
                        s("foo"),
                        vec![vec![0, 0], vec![1, 0]],
                        vec![ftd::Element::Column(external_children)],
                    )),
                    open: (None, Some(s("main-container.foo"))),
                    ..Default::default()
                },
                ..Default::default()
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component desktop:
                component: ftd.column
                optional string $id:
                id: $id

                --- ftd.column:
                id: foo

                -- component mobile:
                component: ftd.column
                optional string $id:
                id: $id

                --- ftd.column:
                id: foo

                -- $is-mobile: true

                -- component page:
                component: ftd.column
                open: main-container.foo

                --- desktop:
                if: not $is-mobile
                id: main-container

                --- mobile:
                if: $is-mobile
                id: main-container

                -- page:

                -- ftd.text: hello

                -- ftd.text: hello again
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn deep_nested_open_container_call() {
        let mut nested_external_children = super::default_column();
        nested_external_children.container.children = vec![
            ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("hello"),
                line: true,
                ..Default::default()
            }),
            ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("hello again"),
                line: true,
                ..Default::default()
            }),
        ];

        let mut external_children = super::default_column();
        external_children.container.children = vec![ftd::Element::Column(ftd::Column {
            container: ftd::Container {
                children: vec![ftd::Element::Row(ftd::Row {
                    container: ftd::Container {
                        children: vec![ftd::Element::Column(ftd::Column {
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(s("id@0,0,0,0"), s("foo"))])
                                    .collect(),
                                data_id: Some(s("foo")),
                                ..Default::default()
                            },
                            ..Default::default()
                        })],
                        ..Default::default()
                    },
                    common: ftd::Common {
                        data_id: Some(s("desktop-container")),
                        ..Default::default()
                    },
                })],
                external_children: Some((
                    s("desktop-container"),
                    vec![vec![0]],
                    vec![ftd::Element::Column(nested_external_children)],
                )),
                open: (None, Some(s("desktop-container"))),
                ..Default::default()
            },
            ..Default::default()
        })];

        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Row(ftd::Row {
                                    container: ftd::Container {
                                        children: vec![ftd::Element::Column(ftd::Column {
                                            common: ftd::Common {
                                                locals: std::array::IntoIter::new([(
                                                    s("id@0,0,0,0"),
                                                    s("foo"),
                                                )])
                                                .collect(),
                                                data_id: Some(s("foo")),
                                                ..Default::default()
                                            },
                                            ..Default::default()
                                        })],
                                        ..Default::default()
                                    },
                                    common: ftd::Common {
                                        data_id: Some(s("desktop-container")),
                                        ..Default::default()
                                    },
                                })],
                                external_children: Some((
                                    s("desktop-container"),
                                    vec![vec![0]],
                                    vec![],
                                )),
                                open: (None, Some(s("desktop-container"))),
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(
                                    s("id@0,0"),
                                    s("main-container"),
                                )])
                                .collect(),
                                condition: Some(ftd::Condition {
                                    variable: s("foo/bar#is-mobile"),
                                    value: s("false"),
                                }),
                                data_id: Some(s("main-container")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Row(ftd::Row {
                                    container: ftd::Container {
                                        children: vec![ftd::Element::Column(ftd::Column {
                                            common: ftd::Common {
                                                locals: std::array::IntoIter::new([(
                                                    s("id@0,1,0,0"),
                                                    s("foo"),
                                                )])
                                                .collect(),
                                                data_id: Some(s("foo")),
                                                ..Default::default()
                                            },
                                            ..Default::default()
                                        })],
                                        ..Default::default()
                                    },
                                    common: ftd::Common {
                                        data_id: Some(s("mobile-container")),
                                        ..Default::default()
                                    },
                                })],
                                external_children: Some((
                                    s("mobile-container"),
                                    vec![vec![0]],
                                    vec![],
                                )),
                                open: (None, Some(s("mobile-container"))),
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(
                                    s("id@0,1"),
                                    s("main-container"),
                                )])
                                .collect(),
                                condition: Some(ftd::Condition {
                                    variable: s("foo/bar#is-mobile"),
                                    value: s("true"),
                                }),
                                is_not_visible: true,
                                data_id: Some(s("main-container")),
                                ..Default::default()
                            },
                        }),
                    ],
                    external_children: Some((
                        s("foo"),
                        vec![vec![0, 0, 0], vec![1, 0, 0]],
                        vec![ftd::Element::Column(external_children)],
                    )),
                    open: (None, Some(s("main-container.foo"))),
                    ..Default::default()
                },
                ..Default::default()
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component ft_container:
                component: ftd.column
                optional string $id:
                id: $id

                -- component ft_container_mobile:
                component: ftd.column
                optional string $id:
                id: $id


                -- component desktop:
                component: ftd.column
                open: desktop-container
                optional string $id:
                id: $id

                --- ftd.row:
                id: desktop-container

                --- ft_container:
                id: foo



                -- component mobile:
                component: ftd.column
                open: mobile-container
                optional string $id:
                id: $id

                --- ftd.row:
                id: mobile-container

                --- ft_container_mobile:
                id: foo


                -- $is-mobile: false


                -- component page:
                component: ftd.column
                open: main-container.foo

                --- desktop:
                if: not $is-mobile
                id: main-container

                --- container: ftd.main

                --- mobile:
                if: $is-mobile
                id: main-container



                -- page:

                -- desktop:

                -- ftd.text: hello

                -- ftd.text: hello again

                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn invalid_deep_open_container() {
        let mut external_children = super::default_column();
        external_children.container.children = vec![
            ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("hello"),
                line: true,
                ..Default::default()
            }),
            ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("hello again"),
                line: true,
                ..Default::default()
            }),
        ];

        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Column(ftd::Column {
                        container: ftd::Container {
                            children: vec![
                                ftd::Element::Column(ftd::Column {
                                    container: ftd::Container {
                                        children: vec![ftd::Element::Column(ftd::Column {
                                            container: ftd::Container {
                                                children: vec![],
                                                ..Default::default()
                                            },
                                            common: ftd::Common {
                                                data_id: Some(s("main-container")),
                                                ..Default::default()
                                            },
                                        })],
                                        ..Default::default()
                                    },
                                    common: ftd::Common {
                                        condition: Some(ftd::Condition {
                                            variable: s("foo/bar#is-mobile"),
                                            value: s("false"),
                                        }),
                                        is_not_visible: true,
                                        ..Default::default()
                                    },
                                }),
                                ftd::Element::Column(ftd::Column {
                                    container: ftd::Container {
                                        children: vec![ftd::Element::Column(ftd::Column {
                                            common: ftd::Common {
                                                data_id: Some(s("main-container")),
                                                ..Default::default()
                                            },
                                            ..Default::default()
                                        })],
                                        ..Default::default()
                                    },
                                    common: ftd::Common {
                                        condition: Some(ftd::Condition {
                                            variable: s("foo/bar#is-mobile"),
                                            value: s("true"),
                                        }),
                                        ..Default::default()
                                    },
                                }),
                            ],
                            ..Default::default()
                        },
                        common: ftd::Common {
                            data_id: Some(s("start")),
                            ..Default::default()
                        },
                    })],
                    external_children: Some((
                        s("main-container"),
                        vec![],
                        vec![ftd::Element::Column(external_children)],
                    )),
                    open: (None, Some(s("main-container"))),
                    ..Default::default()
                },
                ..Default::default()
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component desktop:
                component: ftd.column
                optional string $id:
                id: $id

                --- ftd.column:
                id: main-container

                -- component mobile:
                component: ftd.column
                optional string $id:
                id: $id

                --- ftd.column:
                id: main-container

                -- $is-mobile: true

                -- component page:
                component: ftd.column
                open: main-container

                --- ftd.column:
                id: start

                --- desktop:
                if: not $is-mobile

                --- container: start

                --- mobile:
                if: $is-mobile

                -- page:

                -- ftd.text: hello

                -- ftd.text: hello again
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn open_container_id_1() {
        let mut main = self::default_column();
        main.container.children.push(ftd::Element::Row(ftd::Row {
            common: ftd::Common {
                data_id: Some(s("r1")),
                id: Some(s("r1")),
                ..Default::default()
            },
            container: ftd::Container {
                open: (Some(false), None),
                ..Default::default()
            },
        }));
        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                external_children: Default::default(),
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("hello"),
                        line: true,
                        ..Default::default()
                    }),
                    ftd::Element::Row(ftd::Row {
                        container: ftd::Container {
                            open: (Some(false), None),
                            ..Default::default()
                        },
                        common: ftd::Common {
                            data_id: Some(s("r3")),
                            id: Some(s("r3")),
                            ..Default::default()
                        },
                    }),
                ],
                open: (Some(true), None),
                ..Default::default()
            },
            common: ftd::Common {
                data_id: Some(s("r2")),
                id: Some(s("r2")),
                ..Default::default()
            },
        }));
        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- ftd.row:
                id: r1
                open: false

                -- ftd.row:
                id: r2
                open: true

                --- ftd.text: hello

                -- ftd.row:
                id: r3
                open: false
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_bag, super::default_bag());
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn submit() {
        let mut main = super::default_column();

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello"),
            line: true,
            common: ftd::Common {
                submit: Some("https://httpbin.org/post?x=10".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }));

        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- ftd.text: hello
                submit: https://httpbin.org/post?x=10
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_bag, super::default_bag());
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn basic_loop_on_record_1() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("hello"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@0")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("world"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@body@0")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("body@0"), s("world")),
                    (s("name@0"), s("hello")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("Arpita Jaiswal"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@1")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown("Arpita is developer at Fifthtry"),
                        common: ftd::Common {
                            reference: Some(s("@body@1")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("body@1"), s("Arpita is developer at Fifthtry")),
                    (s("name@1"), s("Arpita Jaiswal")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("Amit Upadhyay"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@2")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown("Amit is CEO of FifthTry."),
                        common: ftd::Common {
                            reference: Some(s("@body@2")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("body@2"), s("Amit is CEO of FifthTry.")),
                    (s("name@2"), s("Amit Upadhyay")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.row".to_string(),
                full_name: s("foo/bar#foo"),
                arguments: std::array::IntoIter::new([
                    (s("body"), crate::p2::Kind::string()),
                    (s("name"), crate::p2::Kind::caption()),
                ])
                .collect(),
                instructions: vec![
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("text"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: "name".to_string(),
                                        kind: crate::p2::Kind::caption_or_body(),
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("text"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: "body".to_string(),
                                        kind: crate::p2::Kind::caption_or_body(),
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                ..Default::default()
            }),
        );

        bag.insert(
            "foo/bar#get".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "get".to_string(),
                value: crate::Value::String {
                    text: "world".to_string(),
                    source: crate::TextSource::Caption,
                },
                conditions: vec![],
            }),
        );

        bag.insert(
            "foo/bar#name".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "name".to_string(),
                value: crate::Value::String {
                    text: "Arpita Jaiswal".to_string(),
                    source: crate::TextSource::Caption,
                },
                conditions: vec![],
            }),
        );

        bag.insert(
            "foo/bar#people".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "foo/bar#people".to_string(),
                value: crate::Value::List {
                    data: vec![
                        crate::Value::Record {
                            name: "foo/bar#person".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("bio"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "Arpita is developer at Fifthtry".to_string(),
                                            source: crate::TextSource::Body,
                                        },
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::PropertyValue::Reference {
                                        name: "foo/bar#name".to_string(),
                                        kind: crate::p2::Kind::caption(),
                                    },
                                ),
                            ])
                            .collect(),
                        },
                        crate::Value::Record {
                            name: "foo/bar#person".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("bio"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "Amit is CEO of FifthTry.".to_string(),
                                            source: crate::TextSource::Body,
                                        },
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "Amit Upadhyay".to_string(),
                                            source: crate::TextSource::Caption,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                    ],
                    kind: crate::p2::Kind::Record {
                        name: "foo/bar#person".to_string(),
                    },
                },
                conditions: vec![],
            }),
        );

        bag.insert(
            "foo/bar#person".to_string(),
            crate::p2::Thing::Record(crate::p2::Record {
                name: "foo/bar#person".to_string(),
                fields: std::array::IntoIter::new([
                    (s("bio"), crate::p2::Kind::body()),
                    (s("name"), crate::p2::Kind::caption()),
                ])
                .collect(),
                instances: Default::default(),
            }),
        );

        p!(
            "
            -- component foo:
            component: ftd.row
            caption $name:
            string $body:

            --- ftd.text: $name

            --- ftd.text: $body

            -- record person:
            caption name:
            body bio:

            -- person list $people:

            -- $name: Arpita Jaiswal

            -- $people: $name

            Arpita is developer at Fifthtry

            -- $people: Amit Upadhyay

            Amit is CEO of FifthTry.

            -- $get: world

            -- foo: hello
            body: $get

            -- foo: $obj.name
            $loop$: $people as $obj
            body: $obj.bio
            ",
            (bag, main),
        );
    }

    #[test]
    fn basic_loop_on_record_with_if_condition() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Null);

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("Amit Upadhyay"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@1")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown("Amit is CEO of FifthTry."),
                        common: ftd::Common {
                            reference: Some(s("@body@1")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@1"), s("Amit Upadhyay")),
                    (s("body@1"), s("Amit is CEO of FifthTry.")),
                ])
                .collect(),
                ..Default::default()
            },
            ..Default::default()
        }));

        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.row".to_string(),
                full_name: s("foo/bar#foo"),
                arguments: std::array::IntoIter::new([
                    (s("body"), crate::p2::Kind::string()),
                    (s("name"), crate::p2::Kind::caption()),
                ])
                .collect(),
                instructions: vec![
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("text"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: "name".to_string(),
                                        kind: crate::p2::Kind::caption_or_body(),
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("text"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: "body".to_string(),
                                        kind: crate::p2::Kind::caption_or_body(),
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                ..Default::default()
            }),
        );

        bag.insert(
            "foo/bar#people".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "foo/bar#people".to_string(),
                value: crate::Value::List {
                    data: vec![
                        crate::Value::Record {
                            name: "foo/bar#person".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("bio"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "Arpita is developer at Fifthtry".to_string(),
                                            source: crate::TextSource::Body,
                                        },
                                    },
                                ),
                                (
                                    s("ceo"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::Boolean { value: false },
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "Arpita Jaiswal".to_string(),
                                            source: crate::TextSource::Caption,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                        crate::Value::Record {
                            name: "foo/bar#person".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("bio"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "Amit is CEO of FifthTry.".to_string(),
                                            source: crate::TextSource::Body,
                                        },
                                    },
                                ),
                                (
                                    s("ceo"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::Boolean { value: true },
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "Amit Upadhyay".to_string(),
                                            source: crate::TextSource::Caption,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                    ],
                    kind: crate::p2::Kind::Record {
                        name: "foo/bar#person".to_string(),
                    },
                },
                conditions: vec![],
            }),
        );

        bag.insert(
            "foo/bar#person".to_string(),
            crate::p2::Thing::Record(crate::p2::Record {
                name: "foo/bar#person".to_string(),
                fields: std::array::IntoIter::new([
                    (s("bio"), crate::p2::Kind::body()),
                    (s("name"), crate::p2::Kind::caption()),
                    (s("ceo"), crate::p2::Kind::boolean()),
                ])
                .collect(),
                instances: Default::default(),
            }),
        );

        p!(
            "
            -- component foo:
            component: ftd.row
            caption $name:
            string $body:

            --- ftd.text: $name

            --- ftd.text: $body

            -- record person:
            caption name:
            body bio:
            boolean ceo:

            -- person list $people:

            -- $people: Arpita Jaiswal
            ceo: false

            Arpita is developer at Fifthtry

            -- $people: Amit Upadhyay
            ceo: true

            Amit is CEO of FifthTry.

            -- foo: $obj.name
            $loop$: $people as $obj
            if: $obj.ceo
            body: $obj.bio
            ",
            (bag, main),
        );
    }

    #[test]
    fn basic_loop_on_string() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("Arpita"),
            line: true,
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("Asit"),
            line: true,
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("Sourabh"),
            line: true,
            ..Default::default()
        }));

        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#people".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "foo/bar#people".to_string(),
                value: crate::Value::List {
                    data: vec![
                        crate::Value::String {
                            text: "Arpita".to_string(),
                            source: crate::TextSource::Caption,
                        },
                        crate::Value::String {
                            text: "Asit".to_string(),
                            source: crate::TextSource::Caption,
                        },
                        crate::Value::String {
                            text: "Sourabh".to_string(),
                            source: crate::TextSource::Caption,
                        },
                    ],
                    kind: crate::p2::Kind::string(),
                },
                conditions: vec![],
            }),
        );
        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- string list $people:

                -- $people: Arpita

                -- $people: Asit

                -- $people: Sourabh

                -- ftd.text: $obj
                $loop$: $people as $obj
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_bag, bag);
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn loop_inside_subsection() {
        let mut main = super::default_column();
        let mut col = ftd::Column {
            ..Default::default()
        };

        col.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("Arpita Jaiswal"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@0,0")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown("Arpita is developer at Fifthtry"),
                        common: ftd::Common {
                            reference: Some(s("@body@0,0")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("body@0,0"), s("Arpita is developer at Fifthtry")),
                    (s("name@0,0"), s("Arpita Jaiswal")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        col.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("Amit Upadhyay"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@0,1")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown("Amit is CEO of FifthTry."),
                        common: ftd::Common {
                            reference: Some(s("@body@0,1")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("body@0,1"), s("Amit is CEO of FifthTry.")),
                    (s("name@0,1"), s("Amit Upadhyay")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        main.container.children.push(ftd::Element::Column(col));

        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(crate::Component {
                root: "ftd.row".to_string(),
                full_name: s("foo/bar#foo"),
                arguments: std::array::IntoIter::new([
                    (s("body"), crate::p2::Kind::string()),
                    (s("name"), crate::p2::Kind::caption()),
                ])
                .collect(),
                instructions: vec![
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("text"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: "name".to_string(),
                                        kind: crate::p2::Kind::caption_or_body(),
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("text"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: "body".to_string(),
                                        kind: crate::p2::Kind::caption_or_body(),
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                invocations: vec![
                    std::array::IntoIter::new([
                        (
                            s("body"),
                            crate::Value::String {
                                text: s("Arpita is developer at Fifthtry"),
                                source: crate::TextSource::Body,
                            },
                        ),
                        (
                            s("name"),
                            crate::Value::String {
                                text: s("Arpita Jaiswal"),
                                source: crate::TextSource::Caption,
                            },
                        ),
                    ])
                    .collect(),
                    std::array::IntoIter::new([
                        (
                            s("body"),
                            crate::Value::String {
                                text: s("Amit is CEO of FifthTry."),
                                source: crate::TextSource::Body,
                            },
                        ),
                        (
                            s("name"),
                            crate::Value::String {
                                text: s("Amit Upadhyay"),
                                source: crate::TextSource::Caption,
                            },
                        ),
                    ])
                    .collect(),
                ],
                ..Default::default()
            }),
        );

        bag.insert(
            "foo/bar#people".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "foo/bar#people".to_string(),
                value: crate::Value::List {
                    data: vec![
                        crate::Value::Record {
                            name: "foo/bar#person".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("bio"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "Arpita is developer at Fifthtry".to_string(),
                                            source: crate::TextSource::Body,
                                        },
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "Arpita Jaiswal".to_string(),
                                            source: crate::TextSource::Caption,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                        crate::Value::Record {
                            name: "foo/bar#person".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("bio"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "Amit is CEO of FifthTry.".to_string(),
                                            source: crate::TextSource::Body,
                                        },
                                    },
                                ),
                                (
                                    s("name"),
                                    crate::PropertyValue::Value {
                                        value: crate::Value::String {
                                            text: "Amit Upadhyay".to_string(),
                                            source: crate::TextSource::Caption,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                    ],
                    kind: crate::p2::Kind::Record {
                        name: "foo/bar#person".to_string(),
                    },
                },
                conditions: vec![],
            }),
        );

        bag.insert(
            "foo/bar#person".to_string(),
            crate::p2::Thing::Record(crate::p2::Record {
                name: "foo/bar#person".to_string(),
                fields: std::array::IntoIter::new([
                    (s("bio"), crate::p2::Kind::body()),
                    (s("name"), crate::p2::Kind::caption()),
                ])
                .collect(),
                instances: Default::default(),
            }),
        );

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component foo:
                component: ftd.row
                caption $name:
                string $body:

                --- ftd.text: $name

                --- ftd.text: $body

                -- record person:
                caption name:
                body bio:

                -- person list $people:

                -- $people: Arpita Jaiswal

                Arpita is developer at Fifthtry

                -- $people: Amit Upadhyay

                Amit is CEO of FifthTry.

                -- ftd.column:

                --- foo: $obj.name
                $loop$: $people as $obj
                body: $obj.bio
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        // pretty_assertions::assert_eq!(g_bag, bag);
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn basic_processor() {
        let mut main = super::default_column();

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("\"0.1.9\""),
            line: true,
            common: ftd::Common {
                reference: Some(s("foo/bar#test")),
                ..Default::default()
            },
            ..Default::default()
        }));

        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#test".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "test".to_string(),
                value: crate::Value::String {
                    text: "\"0.1.9\"".to_string(),
                    source: crate::TextSource::Header,
                },
                conditions: vec![],
            }),
        );

        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- $test:
                $processor$: read_version_from_cargo_toml

                -- ftd.text: $test
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_bag, bag);
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn basic_processor_that_overwrites() {
        let mut main = super::default_column();

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("\"0.1.9\""),
            line: true,
            common: ftd::Common {
                reference: Some(s("foo/bar#test")),
                ..Default::default()
            },
            ..Default::default()
        }));

        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#test".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "test".to_string(),
                value: crate::Value::String {
                    text: "\"0.1.9\"".to_string(),
                    source: crate::TextSource::Header,
                },
                conditions: vec![],
            }),
        );

        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- $test: yo

                -- $test:
                $processor$: read_version_from_cargo_toml

                -- ftd.text: $test
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_bag, bag);
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn basic_processor_for_list() {
        let mut main = super::default_column();

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("\"ftd\""),
            line: true,
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("\"0.1.9\""),
            line: true,
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("[\"Amit Upadhyay <upadhyay@gmail.com>\"]"),
            line: true,
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("\"2021\""),
            line: true,
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("\"ftd: FifthTry Document Format parser\""),
            line: true,
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("\"MIT\""),
            line: true,
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("\"https://github.com/fifthtry/ftd\""),
            line: true,
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("\"https://ftd.dev\""),
            line: true,
            ..Default::default()
        }));

        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#test".to_string(),
            crate::p2::Thing::Variable(crate::Variable {
                name: "foo/bar#test".to_string(),
                value: crate::Value::List {
                    data: vec![
                        crate::Value::String {
                            text: "\"ftd\"".to_string(),
                            source: crate::TextSource::Header,
                        },
                        crate::Value::String {
                            text: "\"0.1.9\"".to_string(),
                            source: crate::TextSource::Header,
                        },
                        crate::Value::String {
                            text: "[\"Amit Upadhyay <upadhyay@gmail.com>\"]".to_string(),
                            source: crate::TextSource::Header,
                        },
                        crate::Value::String {
                            text: "\"2021\"".to_string(),
                            source: crate::TextSource::Header,
                        },
                        crate::Value::String {
                            text: "\"ftd: FifthTry Document Format parser\"".to_string(),
                            source: crate::TextSource::Header,
                        },
                        crate::Value::String {
                            text: "\"MIT\"".to_string(),
                            source: crate::TextSource::Header,
                        },
                        crate::Value::String {
                            text: "\"https://github.com/fifthtry/ftd\"".to_string(),
                            source: crate::TextSource::Header,
                        },
                        crate::Value::String {
                            text: "\"https://ftd.dev\"".to_string(),
                            source: crate::TextSource::Header,
                        },
                    ],
                    kind: crate::p2::Kind::string(),
                },
                conditions: vec![],
            }),
        );

        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- string list $test:
                $processor$: read_package_from_cargo_toml

                -- ftd.text: $obj
                $loop$: $test as $obj
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_bag, bag);
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn processor_for_list_of_record() {
        let mut main = super::default_column();

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("\"ftd\""),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@0")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("name"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@body@0")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@0"), s("\"ftd\"")),
                    (s("body@0"), s("name")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("\"0.1.9\""),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@1")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("version"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@body@1")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@1"), s("\"0.1.9\"")),
                    (s("body@1"), s("version")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("[\"Amit Upadhyay <upadhyay@gmail.com>\"]"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@2")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("authors"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@body@2")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@2"), s("[\"Amit Upadhyay <upadhyay@gmail.com>\"]")),
                    (s("body@2"), s("authors")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("\"2021\""),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@3")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("edition"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@body@3")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@3"), s("\"2021\"")),
                    (s("body@3"), s("edition")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("\"ftd: FifthTry Document Format parser\""),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@4")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("description"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@body@4")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@4"), s("\"ftd: FifthTry Document Format parser\"")),
                    (s("body@4"), s("description")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("\"MIT\""),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@5")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("license"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@body@5")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@5"), s("\"MIT\"")),
                    (s("body@5"), s("license")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("\"https://github.com/fifthtry/ftd\""),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@6")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("repository"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@body@6")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@6"), s("\"https://github.com/fifthtry/ftd\"")),
                    (s("body@6"), s("repository")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("\"https://ftd.dev\""),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@name@7")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("homepage"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@body@7")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@7"), s("\"https://ftd.dev\"")),
                    (s("body@7"), s("homepage")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        let mut bag = super::default_bag();

        bag.insert(
            "foo/bar#data".to_string(),
            crate::p2::Thing::Record(crate::p2::Record {
                name: "foo/bar#data".to_string(),
                fields: std::array::IntoIter::new([
                    (s("description"), crate::p2::Kind::string()),
                    (s("title"), crate::p2::Kind::string()),
                ])
                .collect(),
                instances: Default::default(),
            }),
        );

        bag.insert(
            "foo/bar#foo".to_string(),
            crate::p2::Thing::Component(ftd::Component {
                root: "ftd.row".to_string(),
                full_name: "foo/bar#foo".to_string(),
                arguments: std::array::IntoIter::new([
                    (s("body"), crate::p2::Kind::string()),
                    (s("name"), crate::p2::Kind::caption()),
                ])
                .collect(),
                instructions: vec![
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("text"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: "name".to_string(),
                                        kind: crate::p2::Kind::caption_or_body(),
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    crate::component::Instruction::ChildComponent {
                        child: crate::component::ChildComponent {
                            is_recursive: false,
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([(
                                s("text"),
                                crate::component::Property {
                                    default: Some(crate::PropertyValue::Variable {
                                        name: "body".to_string(),
                                        kind: crate::p2::Kind::caption_or_body(),
                                    }),
                                    conditions: vec![],
                                },
                            )])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                ..Default::default()
            }),
        );

        bag.insert(
            "foo/bar#test".to_string(),
            crate::p2::Thing::Variable(ftd::Variable {
                name: "foo/bar#test".to_string(),
                value: crate::Value::List {
                    data: vec![
                        crate::Value::Record {
                            name: "foo/bar#data".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("description"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "name".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                                (
                                    s("title"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "\"ftd\"".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                        crate::Value::Record {
                            name: "foo/bar#data".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("description"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "version".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                                (
                                    s("title"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "\"0.1.9\"".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                        crate::Value::Record {
                            name: "foo/bar#data".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("description"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "authors".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                                (
                                    s("title"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "[\"Amit Upadhyay <upadhyay@gmail.com>\"]"
                                                .to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                        crate::Value::Record {
                            name: "foo/bar#data".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("description"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "edition".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                                (
                                    s("title"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "\"2021\"".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                        crate::Value::Record {
                            name: "foo/bar#data".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("description"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "description".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                                (
                                    s("title"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "\"ftd: FifthTry Document Format parser\""
                                                .to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                        crate::Value::Record {
                            name: "foo/bar#data".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("description"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "license".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                                (
                                    s("title"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "\"MIT\"".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                        crate::Value::Record {
                            name: "foo/bar#data".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("description"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "repository".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                                (
                                    s("title"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "\"https://github.com/fifthtry/ftd\"".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                        crate::Value::Record {
                            name: "foo/bar#data".to_string(),
                            fields: std::array::IntoIter::new([
                                (
                                    s("description"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "homepage".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                                (
                                    s("title"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: "\"https://ftd.dev\"".to_string(),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                    ],
                    kind: crate::p2::Kind::Record {
                        name: s("foo/bar#data"),
                    },
                },
                conditions: vec![],
            }),
        );

        p!(
            "
            -- component foo:
            component: ftd.row
            caption $name:
            string $body:

            --- ftd.text: $name

            --- ftd.text: $body

            -- record data:
            string title:
            string description:

            -- data list $test:
            $processor$: read_package_records_from_cargo_toml

            -- foo: $obj.title
            $loop$: $test as $obj
            body: $obj.description
            ",
            (bag, main),
        );
    }

    #[test]
    fn loop_with_tree_structure() {
        let mut main = super::default_column();
        let col = ftd::Element::Column(ftd::Column {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("ab title"),
                        line: true,
                        common: ftd::Common {
                            reference: Some(s("@toc.title")),
                            link: Some(s("ab link")),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Column(ftd::Column {
                        container: ftd::Container {
                            children: vec![ftd::Element::Text(ftd::Text {
                                text: ftd::markdown_line("aa title"),
                                line: true,
                                common: ftd::Common {
                                    reference: Some(s("@toc.title")),
                                    link: Some(s("aa link")),
                                    ..Default::default()
                                },
                                ..Default::default()
                            })],
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ftd::Element::Column(ftd::Column {
                        container: ftd::Container {
                            children: vec![ftd::Element::Text(ftd::Text {
                                text: ftd::markdown_line("aaa title"),
                                line: true,
                                common: ftd::Common {
                                    reference: Some(s("@toc.title")),
                                    link: Some(s("aaa link")),
                                    ..Default::default()
                                },
                                ..Default::default()
                            })],
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
            ..Default::default()
        });
        main.container.children.push(col.clone());
        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![col],
                ..Default::default()
            },
            ..Default::default()
        }));

        let mut bag = super::default_bag();

        bag.insert(
            s("foo/bar#aa"),
            crate::p2::Thing::Variable(ftd::Variable {
                name: s("foo/bar#aa"),
                value: ftd::Value::List {
                    data: vec![
                        ftd::Value::Record {
                            name: s("foo/bar#toc-record"),
                            fields: std::array::IntoIter::new([
                                (
                                    s("children"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::List {
                                            data: vec![],
                                            kind: crate::p2::Kind::Record {
                                                name: s("foo/bar#toc-record"),
                                            },
                                        },
                                    },
                                ),
                                (
                                    s("link"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: s("aa link"),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                                (
                                    s("title"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: s("aa title"),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                        ftd::Value::Record {
                            name: s("foo/bar#toc-record"),
                            fields: std::array::IntoIter::new([
                                (
                                    s("children"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::List {
                                            data: vec![],
                                            kind: crate::p2::Kind::Record {
                                                name: s("foo/bar#toc-record"),
                                            },
                                        },
                                    },
                                ),
                                (
                                    s("link"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: s("aaa link"),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                                (
                                    s("title"),
                                    crate::PropertyValue::Value {
                                        value: crate::variable::Value::String {
                                            text: s("aaa title"),
                                            source: crate::TextSource::Header,
                                        },
                                    },
                                ),
                            ])
                            .collect(),
                        },
                    ],
                    kind: crate::p2::Kind::Record {
                        name: s("foo/bar#toc-record"),
                    },
                },
                conditions: vec![],
            }),
        );

        bag.insert(
            s("foo/bar#toc"),
            crate::p2::Thing::Variable(ftd::Variable {
                name: s("foo/bar#toc"),
                value: ftd::Value::List {
                    data: vec![ftd::Value::Record {
                        name: s("foo/bar#toc-record"),
                        fields: std::array::IntoIter::new([
                            (
                                s("children"),
                                crate::PropertyValue::Value {
                                    value: crate::variable::Value::List {
                                        data: vec![
                                            ftd::Value::Record {
                                                name: s("foo/bar#toc-record"),
                                                fields: std::array::IntoIter::new([
                                                    (
                                                        s("children"),
                                                        crate::PropertyValue::Value {
                                                            value: crate::variable::Value::List {
                                                                data: vec![],
                                                                kind: crate::p2::Kind::Record {
                                                                    name: s("foo/bar#toc-record"),
                                                                },
                                                            },
                                                        },
                                                    ),
                                                    (
                                                        s("link"),
                                                        crate::PropertyValue::Value {
                                                            value: crate::variable::Value::String {
                                                                text: s("aa link"),
                                                                source: crate::TextSource::Header,
                                                            },
                                                        },
                                                    ),
                                                    (
                                                        s("title"),
                                                        crate::PropertyValue::Value {
                                                            value: crate::variable::Value::String {
                                                                text: s("aa title"),
                                                                source: crate::TextSource::Header,
                                                            },
                                                        },
                                                    ),
                                                ])
                                                .collect(),
                                            },
                                            ftd::Value::Record {
                                                name: s("foo/bar#toc-record"),
                                                fields: std::array::IntoIter::new([
                                                    (
                                                        s("children"),
                                                        crate::PropertyValue::Value {
                                                            value: crate::variable::Value::List {
                                                                data: vec![],
                                                                kind: crate::p2::Kind::Record {
                                                                    name: s("foo/bar#toc-record"),
                                                                },
                                                            },
                                                        },
                                                    ),
                                                    (
                                                        s("link"),
                                                        crate::PropertyValue::Value {
                                                            value: crate::variable::Value::String {
                                                                text: s("aaa link"),
                                                                source: crate::TextSource::Header,
                                                            },
                                                        },
                                                    ),
                                                    (
                                                        s("title"),
                                                        crate::PropertyValue::Value {
                                                            value: crate::variable::Value::String {
                                                                text: s("aaa title"),
                                                                source: crate::TextSource::Header,
                                                            },
                                                        },
                                                    ),
                                                ])
                                                .collect(),
                                            },
                                        ],
                                        kind: crate::p2::Kind::Record {
                                            name: s("foo/bar#toc-record"),
                                        },
                                    },
                                },
                            ),
                            (
                                s("link"),
                                crate::PropertyValue::Value {
                                    value: crate::variable::Value::String {
                                        text: s("ab link"),
                                        source: crate::TextSource::Header,
                                    },
                                },
                            ),
                            (
                                s("title"),
                                crate::PropertyValue::Value {
                                    value: crate::variable::Value::String {
                                        text: s("ab title"),
                                        source: crate::TextSource::Header,
                                    },
                                },
                            ),
                        ])
                        .collect(),
                    }],
                    kind: crate::p2::Kind::Record {
                        name: s("foo/bar#toc-record"),
                    },
                },
                conditions: vec![],
            }),
        );

        bag.insert(
            s("foo/bar#toc"),
            crate::p2::Thing::Component(ftd::Component {
                root: "ftd.column".to_string(),
                full_name: "foo/bar#toc-item".to_string(),
                arguments: std::array::IntoIter::new([(
                    s("toc"),
                    crate::p2::Kind::Record {
                        name: "foo/bar#toc-record".to_string(),
                    },
                )])
                .collect(),
                instructions: vec![
                    Instruction::ChildComponent {
                        child: ftd::ChildComponent {
                            events: vec![],
                            root: "ftd#text".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([
                                (
                                    s("link"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Variable {
                                            name: "toc.link".to_string(),
                                            kind: crate::p2::Kind::Optional {
                                                kind: Box::new(crate::p2::Kind::string()),
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("text"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Variable {
                                            name: "toc.title".to_string(),
                                            kind: crate::p2::Kind::Optional {
                                                kind: Box::new(crate::p2::Kind::caption_or_body()),
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                    Instruction::RecursiveChildComponent {
                        child: ftd::ChildComponent {
                            is_recursive: true,
                            events: vec![],
                            root: "toc-item".to_string(),
                            condition: None,
                            properties: std::array::IntoIter::new([
                                (
                                    s("$loop$"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Variable {
                                            name: "toc.children".to_string(),
                                            kind: crate::p2::Kind::Record {
                                                name: s("foo/bar#toc-record"),
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                                (
                                    s("toc"),
                                    crate::component::Property {
                                        default: Some(crate::PropertyValue::Variable {
                                            name: "$loop$".to_string(),
                                            kind: crate::p2::Kind::Record {
                                                name: s("foo/bar#toc-record"),
                                            },
                                        }),
                                        conditions: vec![],
                                    },
                                ),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    },
                ],
                ..Default::default()
            }),
        );

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- record toc-record:
                string title:
                string link:
                toc-record list children:

                -- component toc-item:
                component: ftd.column
                toc-record $toc:

                --- ftd.text: $toc.title
                link: $toc.link

                --- toc-item:
                $loop$: $toc.children as $obj
                toc: $obj

                -- toc-record list $aa:

                -- $aa:
                title: aa title
                link: aa link

                -- $aa:
                title: aaa title
                link: aaa link

                -- toc-record list $toc:

                -- $toc:
                title: ab title
                link: ab link
                children: $aa

                -- component foo:
                component: ftd.row

                --- toc-item:
                $loop$: $toc as $obj
                toc: $obj

                -- toc-item:
                $loop$: $toc as $obj
                toc: $obj

                -- foo:
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        // pretty_assertions::assert_eq!(g_bag, bag);
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn import_check() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![ftd::Element::Text(ftd::Text {
                    text: ftd::markdown_line("Hello World"),
                    line: true,
                    common: ftd::Common {
                        reference: Some(s("hello-world-variable#hello-world")),
                        ..Default::default()
                    },
                    ..Default::default()
                })],
                ..Default::default()
            },
            ..Default::default()
        }));

        let mut bag = super::default_bag();
        bag.insert(
            s("hello-world#foo"),
            crate::p2::Thing::Component(ftd::Component {
                root: s("ftd.row"),
                full_name: s("hello-world#foo"),
                instructions: vec![ftd::Instruction::ChildComponent {
                    child: ftd::ChildComponent {
                        events: vec![],
                        root: s("ftd#text"),
                        condition: None,
                        properties: std::array::IntoIter::new([(
                            s("text"),
                            crate::component::Property {
                                default: Some(crate::PropertyValue::Reference {
                                    name: "hello-world-variable#hello-world".to_string(),
                                    kind: crate::p2::Kind::caption_or_body(),
                                }),
                                conditions: vec![],
                            },
                        )])
                        .collect(),
                        ..Default::default()
                    },
                }],
                invocations: vec![],
                ..Default::default()
            }),
        );
        bag.insert(
            s("hello-world-variable#hello-world"),
            crate::p2::Thing::Variable(ftd::Variable {
                name: s("hello-world"),
                value: ftd::Value::String {
                    text: s("Hello World"),
                    source: ftd::TextSource::Caption,
                },
                conditions: vec![],
            }),
        );

        p!(
            "
            -- import: hello-world as hw

            -- hw.foo:
            ",
            (bag, main),
        );
    }

    #[test]
    fn argument_with_default_value() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello world"),
            line: true,
            size: Some(10),
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@0"), s("hello world")),
                    (s("size@0"), s("10")),
                ])
                .collect(),
                reference: Some(s("@name@0")),
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello"),
            line: true,
            size: Some(10),
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@1"), s("hello")),
                    (s("size@1"), s("10")),
                ])
                .collect(),
                reference: Some(s("@name@1")),
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("this is nice"),
            line: true,
            size: Some(20),
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@2"), s("this is nice")),
                    (s("size@2"), s("20")),
                ])
                .collect(),
                reference: Some(s("@name@2")),
                ..Default::default()
            },
            ..Default::default()
        }));

        let mut bag = super::default_bag();
        bag.insert(
            s("foo/bar#foo"),
            crate::p2::Thing::Component(ftd::Component {
                root: s("ftd.text"),
                full_name: s("foo/bar#foo"),
                arguments: std::array::IntoIter::new([
                    (
                        s("name"),
                        crate::p2::Kind::caption().set_default(Some(s("hello world"))),
                    ),
                    (
                        s("size"),
                        crate::p2::Kind::Integer {
                            default: Some(s("10")),
                        },
                    ),
                ])
                .collect(),
                properties: std::array::IntoIter::new([
                    (
                        s("size"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Variable {
                                name: s("size"),
                                kind: crate::p2::Kind::Optional {
                                    kind: Box::from(crate::p2::Kind::Integer {
                                        default: Some(s("10")),
                                    }),
                                },
                            }),
                            conditions: vec![],
                        },
                    ),
                    (
                        s("text"),
                        crate::component::Property {
                            default: Some(crate::PropertyValue::Variable {
                                name: s("name"),
                                kind: crate::p2::Kind::caption_or_body()
                                    .set_default(Some(s("hello world"))),
                            }),
                            conditions: vec![],
                        },
                    ),
                ])
                .collect(),
                invocations: vec![
                    std::array::IntoIter::new([
                        (
                            s("name"),
                            crate::Value::String {
                                text: s("hello world"),
                                source: crate::TextSource::Default,
                            },
                        ),
                        (s("size"), crate::Value::Integer { value: 10 }),
                    ])
                    .collect(),
                    std::array::IntoIter::new([
                        (
                            s("name"),
                            crate::Value::String {
                                text: s("hello"),
                                source: crate::TextSource::Caption,
                            },
                        ),
                        (s("size"), crate::Value::Integer { value: 10 }),
                    ])
                    .collect(),
                    std::array::IntoIter::new([
                        (
                            s("name"),
                            crate::Value::String {
                                text: s("this is nice"),
                                source: crate::TextSource::Caption,
                            },
                        ),
                        (s("size"), crate::Value::Integer { value: 20 }),
                    ])
                    .collect(),
                ],
                ..Default::default()
            }),
        );

        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component foo:
                component: ftd.text
                caption $name: hello world
                integer $size: 10
                text: $name
                size: $size

                -- foo:

                -- foo: hello

                -- foo: this is nice
                size: 20
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_bag, bag);
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn record_with_default_value() {
        let mut bag = super::default_bag();
        bag.insert(
            s("foo/bar#abrar"),
            crate::p2::Thing::Variable(ftd::Variable {
                name: s("abrar"),
                value: ftd::Value::Record {
                    name: s("foo/bar#person"),
                    fields: std::array::IntoIter::new([
                        (
                            s("address"),
                            crate::PropertyValue::Value {
                                value: crate::variable::Value::String {
                                    text: s("Bihar"),
                                    source: crate::TextSource::Default,
                                },
                            },
                        ),
                        (
                            s("age"),
                            crate::PropertyValue::Reference {
                                name: s("foo/bar#default-age"),
                                kind: crate::p2::Kind::Integer {
                                    default: Some(s("$default-age")),
                                },
                            },
                        ),
                        (
                            s("bio"),
                            crate::PropertyValue::Value {
                                value: crate::variable::Value::String {
                                    text: s("Software developer working at fifthtry."),
                                    source: crate::TextSource::Body,
                                },
                            },
                        ),
                        (
                            s("name"),
                            crate::PropertyValue::Reference {
                                name: s("foo/bar#abrar-name"),
                                kind: crate::p2::Kind::caption(),
                            },
                        ),
                        (
                            s("size"),
                            crate::PropertyValue::Value {
                                value: crate::variable::Value::Integer { value: 10 },
                            },
                        ),
                    ])
                    .collect(),
                },
                conditions: vec![],
            }),
        );
        bag.insert(
            s("foo/bar#abrar-name"),
            crate::p2::Thing::Variable(ftd::Variable {
                name: s("abrar-name"),
                value: crate::variable::Value::String {
                    text: s("Abrar Khan"),
                    source: crate::TextSource::Caption,
                },
                conditions: vec![],
            }),
        );
        bag.insert(
            s("foo/bar#default-age"),
            crate::p2::Thing::Variable(ftd::Variable {
                name: s("default-age"),
                value: crate::variable::Value::Integer { value: 20 },
                conditions: vec![],
            }),
        );
        bag.insert(
            s("foo/bar#person"),
            crate::p2::Thing::Record(ftd::p2::Record {
                name: s("foo/bar#person"),
                fields: std::array::IntoIter::new([
                    (
                        s("address"),
                        crate::p2::Kind::string().set_default(Some(s("Bihar"))),
                    ),
                    (
                        s("age"),
                        crate::p2::Kind::Integer {
                            default: Some(s("$default-age")),
                        },
                    ),
                    (
                        s("bio"),
                        crate::p2::Kind::body().set_default(Some(s("Some Bio"))),
                    ),
                    (s("name"), crate::p2::Kind::caption()),
                    (
                        s("size"),
                        crate::p2::Kind::Integer {
                            default: Some(s("10")),
                        },
                    ),
                ])
                .collect(),
                instances: Default::default(),
            }),
        );

        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown("Software developer working at fifthtry."),
            size: Some(20),
            common: ftd::Common {
                reference: Some(s("abrar.bio")),
                ..Default::default()
            },
            ..Default::default()
        }));

        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- $default-age: 20

                -- record person:
                caption name:
                string address: Bihar
                body bio: Some Bio
                integer age: $default-age
                integer size: 10

                -- $abrar-name: Abrar Khan

                -- person $abrar: $abrar-name

                Software developer working at fifthtry.

                -- ftd.text: $abrar.bio
                size: $abrar.age
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_bag, bag);
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn default_with_reference() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![ftd::Element::Text(ftd::Text {
                    text: ftd::markdown_line("Arpita"),
                    line: true,
                    size: Some(10),
                    common: ftd::Common {
                        reference: Some(s("@name@0")),
                        ..Default::default()
                    },
                    ..Default::default()
                })],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@0"), s("Arpita")),
                    (s("text-size@0"), s("10")),
                ])
                .collect(),
                ..Default::default()
            },
        }));
        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![ftd::Element::Text(ftd::Text {
                    text: ftd::markdown_line("Amit Upadhayay"),
                    line: true,
                    size: Some(20),
                    common: ftd::Common {
                        reference: Some(s("@name@1")),
                        ..Default::default()
                    },
                    ..Default::default()
                })],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@1"), s("Amit Upadhayay")),
                    (s("text-size@1"), s("20")),
                ])
                .collect(),
                ..Default::default()
            },
        }));

        let mut bag = super::default_bag();
        bag.insert(
            s("foo/bar#default-name"),
            crate::p2::Thing::Variable(ftd::Variable {
                name: s("default-name"),
                value: crate::Value::String {
                    text: s("Arpita"),
                    source: crate::TextSource::Caption,
                },
                conditions: vec![],
            }),
        );
        bag.insert(
            s("foo/bar#default-size"),
            crate::p2::Thing::Variable(ftd::Variable {
                name: s("default-size"),
                value: crate::Value::Integer { value: 10 },
                conditions: vec![],
            }),
        );
        bag.insert(
            s("foo/bar#foo"),
            crate::p2::Thing::Component(ftd::Component {
                root: s("ftd.row"),
                full_name: s("foo/bar#foo"),
                arguments: std::array::IntoIter::new([
                    (
                        s("name"),
                        crate::p2::Kind::string().set_default(Some(s("$default-name"))),
                    ),
                    (
                        s("text-size"),
                        crate::p2::Kind::Integer {
                            default: Some(s("$default-size")),
                        },
                    ),
                ])
                .collect(),
                instructions: vec![ftd::Instruction::ChildComponent {
                    child: ftd::ChildComponent {
                        events: vec![],
                        root: s("ftd#text"),
                        condition: None,
                        properties: std::array::IntoIter::new([
                            (
                                s("size"),
                                crate::component::Property {
                                    default: Some(ftd::PropertyValue::Variable {
                                        name: s("text-size"),
                                        kind: ftd::p2::Kind::Optional {
                                            kind: Box::new(ftd::p2::Kind::Integer {
                                                default: Some(s("$default-size")),
                                            }),
                                        },
                                    }),
                                    conditions: vec![],
                                },
                            ),
                            (
                                s("text"),
                                crate::component::Property {
                                    default: Some(ftd::PropertyValue::Variable {
                                        name: s("name"),
                                        kind: ftd::p2::Kind::caption_or_body()
                                            .set_default(Some(s("$default-name"))),
                                    }),
                                    conditions: vec![],
                                },
                            ),
                        ])
                        .collect(),
                        ..Default::default()
                    },
                }],
                kernel: false,
                ..Default::default()
            }),
        );

        p!(
            "
            -- $default-name: Arpita

            -- $default-size: 10

            -- component foo:
            component: ftd.row
            string $name: $default-name
            integer $text-size: $default-size

            --- ftd.text: $name
            size: $text-size

            -- foo:

            -- foo:
            name: Amit Upadhayay
            text-size: 20
            ",
            (bag, main),
        );
    }

    #[test]
    fn or_type_with_default_value() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("Amit Upadhyay"),
            line: true,
            common: ftd::Common {
                reference: Some(s("amitu.name")),
                ..Default::default()
            },
            ..Default::default()
        }));
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("1000"),
            line: true,
            common: ftd::Common {
                reference: Some(s("amitu.phone")),
                ..Default::default()
            },
            ..Default::default()
        }));
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("John Doe"),
            line: true,
            size: Some(50),
            common: ftd::Common {
                reference: Some(s("acme.contact")),
                ..Default::default()
            },
            ..Default::default()
        }));

        let mut bag = super::default_bag();
        bag.insert(
            s("foo/bar#acme"),
            ftd::p2::Thing::Variable(ftd::Variable {
                name: s("acme"),
                value: ftd::Value::OrType {
                    name: s("foo/bar#lead"),
                    variant: s("company"),
                    fields: std::array::IntoIter::new([
                        (
                            s("contact"),
                            ftd::PropertyValue::Value {
                                value: ftd::Value::String {
                                    text: s("John Doe"),
                                    source: ftd::TextSource::Header,
                                },
                            },
                        ),
                        (
                            s("fax"),
                            ftd::PropertyValue::Value {
                                value: ftd::Value::String {
                                    text: s("+1-234-567890"),
                                    source: ftd::TextSource::Header,
                                },
                            },
                        ),
                        (
                            s("name"),
                            ftd::PropertyValue::Value {
                                value: ftd::Value::String {
                                    text: s("Acme Inc."),
                                    source: ftd::TextSource::Caption,
                                },
                            },
                        ),
                        (
                            s("no-of-employees"),
                            ftd::PropertyValue::Value {
                                value: ftd::Value::Integer { value: 50 },
                            },
                        ),
                    ])
                    .collect(),
                },
                conditions: vec![],
            }),
        );
        bag.insert(
            s("foo/bar#amitu"),
            ftd::p2::Thing::Variable(ftd::Variable {
                name: s("amitu"),
                value: ftd::Value::OrType {
                    name: s("foo/bar#lead"),
                    variant: s("individual"),
                    fields: std::array::IntoIter::new([
                        (
                            s("name"),
                            ftd::PropertyValue::Value {
                                value: ftd::Value::String {
                                    text: s("Amit Upadhyay"),
                                    source: ftd::TextSource::Caption,
                                },
                            },
                        ),
                        (
                            s("phone"),
                            ftd::PropertyValue::Reference {
                                name: s("foo/bar#default-phone"),
                                kind: ftd::p2::Kind::string()
                                    .set_default(Some(s("$default-phone"))),
                            },
                        ),
                    ])
                    .collect(),
                },
                conditions: vec![],
            }),
        );
        bag.insert(
            s("foo/bar#default-phone"),
            ftd::p2::Thing::Variable(ftd::Variable {
                name: s("default-phone"),
                value: ftd::Value::String {
                    text: s("1000"),
                    source: ftd::TextSource::Caption,
                },
                conditions: vec![],
            }),
        );
        bag.insert(
            s("foo/bar#lead"),
            ftd::p2::Thing::OrType(ftd::OrType {
                name: s("foo/bar#lead"),
                variants: vec![
                    ftd::p2::Record {
                        name: s("foo/bar#lead.individual"),
                        fields: std::array::IntoIter::new([
                            (s("name"), ftd::p2::Kind::caption()),
                            (
                                s("phone"),
                                ftd::p2::Kind::string().set_default(Some(s("$default-phone"))),
                            ),
                        ])
                        .collect(),
                        instances: Default::default(),
                    },
                    ftd::p2::Record {
                        name: s("foo/bar#lead.company"),
                        fields: std::array::IntoIter::new([
                            (
                                s("contact"),
                                ftd::p2::Kind::string().set_default(Some(s("1001"))),
                            ),
                            (s("fax"), ftd::p2::Kind::string()),
                            (s("name"), ftd::p2::Kind::caption()),
                            (
                                s("no-of-employees"),
                                ftd::p2::Kind::integer().set_default(Some(s("50"))),
                            ),
                        ])
                        .collect(),
                        instances: Default::default(),
                    },
                ],
            }),
        );

        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- string $default-phone: 1000

                -- or-type lead:

                --- individual:
                caption name:
                string phone: $default-phone

                --- company:
                caption name:
                string contact: 1001
                string fax:
                integer no-of-employees: 50

                -- lead.individual $amitu: Amit Upadhyay

                -- lead.company $acme: Acme Inc.
                contact: John Doe
                fax: +1-234-567890

                -- ftd.text: $amitu.name

                -- ftd.text: $amitu.phone

                -- ftd.text: $acme.contact
                size: $acme.no-of-employees

                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_bag, bag);
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn default_id() {
        let mut main = super::default_column();

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Column(ftd::Column {
                        container: ftd::Container {
                            children: vec![ftd::Element::Row(ftd::Row {
                                container: ftd::Container {
                                    children: vec![ftd::Element::Column(ftd::Column {
                                        container: ftd::Container {
                                            children: vec![ftd::Element::Text(ftd::Text {
                                                text: ftd::markdown_line("hello"),
                                                line: true,
                                                ..Default::default()
                                            })],
                                            ..Default::default()
                                        },
                                        common: ftd::Common {
                                            data_id: Some(s("display-text-id")),
                                            ..Default::default()
                                        },
                                    })],
                                    ..Default::default()
                                },
                                ..Default::default()
                            })],
                            ..Default::default()
                        },
                        common: ftd::Common {
                            data_id: Some(s("inside-page-id")),
                            ..Default::default()
                        },
                    })],
                    ..Default::default()
                },
                ..Default::default()
            }));

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Row(ftd::Row {
                                    container: ftd::Container {
                                        children: vec![ftd::Element::Column(ftd::Column {
                                            container: ftd::Container {
                                                children: vec![ftd::Element::Text(ftd::Text {
                                                    text: ftd::markdown_line("hello"),
                                                    line: true,
                                                    ..Default::default()
                                                })],
                                                ..Default::default()
                                            },
                                            common: ftd::Common {
                                                data_id: Some(s("display-text-id")),
                                                id: Some(s(
                                                    "page-id:inside-page-id:display-text-id",
                                                )),
                                                ..Default::default()
                                            },
                                        })],
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            common: ftd::Common {
                                data_id: Some(s("inside-page-id")),
                                id: Some(s("page-id:inside-page-id")),
                                ..Default::default()
                            },
                        }),
                        ftd::Element::Row(ftd::Row {
                            common: ftd::Common {
                                data_id: Some(s("page-id-row")),
                                id: Some(s("page-id-row")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    data_id: Some(s("page-id")),
                    id: Some(s("page-id")),
                    ..Default::default()
                },
            }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component display-text:
                component: ftd.column

                --- ftd.text: hello


                -- component inside-page:
                component: ftd.column

                --- ftd.row:

                --- display-text:
                id: display-text-id


                -- component page:
                component: ftd.column

                --- inside-page:
                id: inside-page-id

                -- page:

                -- page:
                id: page-id

                -- ftd.row:

                -- container: page-id

                -- ftd.row:
                id: page-id-row

                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn region_h1() {
        let mut main = super::default_column();

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("Heading 31"),
                        line: true,
                        common: ftd::Common {
                            region: Some(ftd::Region::Title),
                            reference: Some(s("@title@0")),
                            ..Default::default()
                        },
                        ..Default::default()
                    })],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([(s("title@0"), s("Heading 31"))]).collect(),
                    region: Some(ftd::Region::H3),
                    id: Some(s("heading-31")),
                    ..Default::default()
                },
            }));

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Heading 11"),
                            line: true,
                            common: ftd::Common {
                                region: Some(ftd::Region::Title),
                                reference: Some(s("@title@1")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![
                                    ftd::Element::Text(ftd::Text {
                                        text: ftd::markdown_line("Heading 21"),
                                        line: true,
                                        common: ftd::Common {
                                            region: Some(ftd::Region::Title),
                                            reference: Some(s("@title@2")),
                                            ..Default::default()
                                        },
                                        ..Default::default()
                                    }),
                                    ftd::Element::Column(ftd::Column {
                                        container: ftd::Container {
                                            children: vec![
                                                ftd::Element::Text(ftd::Text {
                                                    text: ftd::markdown_line("Heading 32"),
                                                    line: true,
                                                    common: ftd::Common {
                                                        region: Some(ftd::Region::Title),
                                                        reference: Some(s("@title@3")),
                                                        ..Default::default()
                                                    },
                                                    ..Default::default()
                                                }),
                                                ftd::Element::Text(ftd::Text {
                                                    text: ftd::markdown_line("hello"),
                                                    line: true,
                                                    ..Default::default()
                                                }),
                                            ],
                                            ..Default::default()
                                        },
                                        common: ftd::Common {
                                            locals: std::array::IntoIter::new([(
                                                s("title@3"),
                                                s("Heading 32"),
                                            )])
                                            .collect(),
                                            region: Some(ftd::Region::H3),
                                            id: Some(s("heading-32")),
                                            ..Default::default()
                                        },
                                    }),
                                ],
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(
                                    s("title@2"),
                                    s("Heading 21"),
                                )])
                                .collect(),
                                region: Some(ftd::Region::H2),
                                id: Some(s("heading-21")),
                                ..Default::default()
                            },
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("Heading 22"),
                                    line: true,
                                    common: ftd::Common {
                                        reference: Some(s("@title@5")),
                                        region: Some(ftd::Region::Title),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(
                                    s("title@5"),
                                    s("Heading 22"),
                                )])
                                .collect(),
                                region: Some(ftd::Region::H2),
                                id: Some(s("heading-22")),
                                ..Default::default()
                            },
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("Heading 23"),
                                    line: true,
                                    common: ftd::Common {
                                        region: Some(ftd::Region::Title),
                                        reference: Some(s("@title@6")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(
                                    s("title@6"),
                                    s("Heading 23"),
                                )])
                                .collect(),
                                region: Some(ftd::Region::H2),
                                id: Some(s("heading-23")),
                                ..Default::default()
                            },
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([(s("title@1"), s("Heading 11"))]).collect(),
                    region: Some(ftd::Region::H1),
                    id: Some(s("heading-11")),
                    ..Default::default()
                },
            }));

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Heading 12"),
                            line: true,
                            common: ftd::Common {
                                reference: Some(s("@title@7")),
                                region: Some(ftd::Region::Title),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("Heading 33"),
                                    line: true,
                                    common: ftd::Common {
                                        reference: Some(s("@title@8")),
                                        region: Some(ftd::Region::Title),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(
                                    s("title@8"),
                                    s("Heading 33"),
                                )])
                                .collect(),
                                region: Some(ftd::Region::H3),
                                id: Some(s("heading-33")),
                                ..Default::default()
                            },
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("Heading 24"),
                                    line: true,
                                    common: ftd::Common {
                                        reference: Some(s("@title@9")),
                                        region: Some(ftd::Region::Title),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(
                                    s("title@9"),
                                    s("Heading 24"),
                                )])
                                .collect(),
                                region: Some(ftd::Region::H2),
                                id: Some(s("heading-24")),
                                ..Default::default()
                            },
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([(s("title@7"), s("Heading 12"))]).collect(),
                    region: Some(ftd::Region::H1),
                    id: Some(s("heading-12")),
                    ..Default::default()
                },
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component h1:
                component: ftd.column
                region: h1
                caption $title:

                --- ftd.text:
                text: $title
                caption $title:
                region: title

                -- component h2:
                component: ftd.column
                region: h2
                caption $title:

                --- ftd.text:
                text: $title
                caption $title:
                region: title

                -- component h3:
                component: ftd.column
                region: h3
                caption $title:

                --- ftd.text:
                text: $title
                caption $title:
                region: title

                -- h3: Heading 31

                -- h1: Heading 11

                -- h2: Heading 21

                -- h3: Heading 32

                -- ftd.text: hello

                -- h2: Heading 22

                -- h2: Heading 23

                -- h1: Heading 12

                -- h3: Heading 33

                -- h2: Heading 24

                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn event_onclick() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Mobile"),
                            line: true,
                            common: ftd::Common {
                                condition: Some(ftd::Condition {
                                    variable: s("foo/bar#mobile"),
                                    value: s("true"),
                                }),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Desktop"),
                            line: true,
                            common: ftd::Common {
                                condition: Some(ftd::Condition {
                                    variable: s("foo/bar#mobile"),
                                    value: s("false"),
                                }),
                                is_not_visible: true,
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                ..Default::default()
            }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("Click Here!"),
            line: true,
            common: ftd::Common {
                events: vec![ftd::Event {
                    name: s("onclick"),
                    action: ftd::Action {
                        action: s("toggle"),
                        target: s("foo/bar#mobile"),
                        parameters: Default::default(),
                    },
                }],
                ..Default::default()
            },
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- $mobile: true

                -- component foo:
                component: ftd.column

                --- ftd.text: Mobile
                if: $mobile

                --- ftd.text: Desktop
                if: not $mobile

                -- foo:

                -- ftd.text: Click Here!
                $event-click$: toggle $mobile
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn event_toggle_with_local_variable() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("Hello"),
            line: true,
            common: ftd::Common {
                locals: std::array::IntoIter::new([
                    (s("name@0"), s("Hello")),
                    (s("open@0"), s("true")),
                ])
                .collect(),
                reference: Some(s("@name@0")),
                condition: Some(ftd::Condition {
                    variable: s("@open@0"),
                    value: s("true"),
                }),
                events: vec![ftd::Event {
                    name: s("onclick"),
                    action: ftd::Action {
                        action: s("toggle"),
                        target: s("@open@0"),
                        parameters: Default::default(),
                    },
                }],
                ..Default::default()
            },
            ..Default::default()
        }));

        let mut bag = super::default_bag();
        bag.insert(
            s("foo/bar#foo"),
            ftd::p2::Thing::Component(ftd::Component {
                root: "ftd.text".to_string(),
                full_name: "foo/bar#foo".to_string(),
                arguments: std::array::IntoIter::new([
                    (s("name"), ftd::p2::Kind::caption()),
                    (
                        s("open"),
                        ftd::p2::Kind::boolean().set_default(Some(s("true"))),
                    ),
                ])
                .collect(),
                properties: std::array::IntoIter::new([(
                    s("text"),
                    ftd::component::Property {
                        default: Some(ftd::PropertyValue::Variable {
                            name: s("name"),
                            kind: ftd::p2::Kind::String {
                                caption: true,
                                body: true,
                                default: None,
                            },
                        }),
                        ..Default::default()
                    },
                )])
                .collect(),
                instructions: vec![],
                events: vec![ftd::p2::Event {
                    name: ftd::p2::EventName::OnClick,
                    action: ftd::p2::Action {
                        action: ftd::p2::ActionKind::Toggle,
                        target: s("@open"),
                        parameters: Default::default(),
                    },
                }],
                condition: Some(ftd::p2::Boolean::Equal {
                    left: ftd::PropertyValue::Variable {
                        name: s("open"),
                        kind: ftd::p2::Kind::boolean().set_default(Some(s("true"))),
                    },
                    right: ftd::PropertyValue::Value {
                        value: crate::variable::Value::Boolean { value: true },
                    },
                }),
                kernel: false,
                invocations: vec![std::array::IntoIter::new([
                    (
                        s("name"),
                        ftd::Value::String {
                            text: s("Hello"),
                            source: ftd::TextSource::Caption,
                        },
                    ),
                    (s("open"), ftd::Value::Boolean { value: true }),
                ])
                .collect()],
                ..Default::default()
            }),
        );

        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component foo:
                component: ftd.text
                caption $name:
                boolean $open: true
                text: $name
                if: $open
                $event-click$: toggle $open

                -- foo: Hello
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
        pretty_assertions::assert_eq!(g_bag, bag);
    }

    #[test]
    fn event_toggle_with_local_variable_for_component() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Click here"),
                            line: true,
                            common: ftd::Common {
                                events: vec![ftd::Event {
                                    name: s("onclick"),
                                    action: ftd::Action {
                                        action: s("toggle"),
                                        target: s("@open@0"),
                                        parameters: Default::default(),
                                    },
                                }],
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Open True"),
                            line: true,
                            common: ftd::Common {
                                condition: Some(ftd::Condition {
                                    variable: s("@open@0"),
                                    value: s("true"),
                                }),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Open False"),
                            line: true,
                            common: ftd::Common {
                                condition: Some(ftd::Condition {
                                    variable: s("@open@0"),
                                    value: s("false"),
                                }),
                                is_not_visible: true,
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([(s("open@0"), s("true"))]).collect(),
                    ..Default::default()
                },
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component foo:
                component: ftd.column
                boolean $open: true

                --- ftd.text: Click here
                $event-click$: toggle $open

                --- ftd.text: Open True
                if: $open

                --- ftd.text: Open False
                if: not $open

                -- foo:
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn event_toggle_for_loop() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("ab title"),
                            line: true,
                            common: ftd::Common {
                                events: vec![ftd::Event {
                                    name: s("onclick"),
                                    action: ftd::Action {
                                        action: s("toggle"),
                                        target: s("@open@0"),
                                        parameters: Default::default(),
                                    },
                                }],
                                reference: Some(s("@toc.title")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("aa title"),
                                    line: true,
                                    common: ftd::Common {
                                        events: vec![ftd::Event {
                                            name: s("onclick"),
                                            action: ftd::Action {
                                                action: s("toggle"),
                                                target: s("@open@0,1"),
                                                parameters: Default::default(),
                                            },
                                        }],
                                        reference: Some(s("@toc.title")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(s("open@0,1"), s("true"))])
                                    .collect(),
                                condition: Some(ftd::Condition {
                                    variable: s("@open@0"),
                                    value: s("true"),
                                }),
                                ..Default::default()
                            },
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("aaa title"),
                                    line: true,
                                    common: ftd::Common {
                                        events: vec![ftd::Event {
                                            name: s("onclick"),
                                            action: ftd::Action {
                                                action: s("toggle"),
                                                target: s("@open@0,2"),
                                                parameters: Default::default(),
                                            },
                                        }],
                                        reference: Some(s("@toc.title")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(s("open@0,2"), s("true"))])
                                    .collect(),
                                condition: Some(ftd::Condition {
                                    variable: s("@open@0"),
                                    value: s("true"),
                                }),
                                ..Default::default()
                            },
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([(s("open@0"), s("true"))]).collect(),
                    ..Default::default()
                },
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- record toc-record:
                string title:
                toc-record list children:

                -- component toc-item:
                component: ftd.column
                toc-record $toc:
                boolean $open: true

                --- ftd.text: $toc.title
                $event-click$: toggle $open

                --- toc-item:
                if: $open
                $loop$: $toc.children as $obj
                toc: $obj

                -- toc-record list $aa:

                -- $aa:
                title: aa title

                -- $aa:
                title: aaa title

                -- toc-record list $toc:

                -- $toc:
                title: ab title
                children: $aa

                -- toc-item:
                $loop$: $toc as $obj
                toc: $obj
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn test_local_variable() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Column(ftd::Column {
                        container: ftd::Container {
                            children: vec![
                                ftd::Element::Column(ftd::Column {
                                    container: ftd::Container {
                                        children: vec![
                                            ftd::Element::Text(ftd::Text {
                                                text: ftd::markdown_line("Click here!"),
                                                line: true,
                                                common: ftd::Common {
                                                    events: vec![ftd::Event {
                                                        name: s("onclick"),
                                                        action: ftd::Action {
                                                            action: s("toggle"),
                                                            target: s("@open@0"),
                                                            parameters: Default::default(),
                                                        },
                                                    }],
                                                    ..Default::default()
                                                },
                                                ..Default::default()
                                            }),
                                            ftd::Element::Text(ftd::Text {
                                                text: ftd::markdown_line("Hello"),
                                                line: true,
                                                ..Default::default()
                                            }),
                                        ],
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }),
                                ftd::Element::Column(ftd::Column {
                                    container: ftd::Container {
                                        children: vec![ftd::Element::Text(ftd::Text {
                                            text: ftd::markdown_line("Hello Bar"),
                                            line: true,
                                            ..Default::default()
                                        })],
                                        ..Default::default()
                                    },
                                    common: ftd::Common {
                                        locals: std::array::IntoIter::new([(
                                            s("open-bar@0,0,1"),
                                            s("true"),
                                        )])
                                        .collect(),
                                        condition: Some(ftd::Condition {
                                            variable: s("@open@0"),
                                            value: s("true"),
                                        }),
                                        ..Default::default()
                                    },
                                }),
                            ],
                            ..Default::default()
                        },
                        common: ftd::Common {
                            data_id: Some(s("foo-id")),
                            ..Default::default()
                        },
                    })],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([(s("open@0"), s("true"))]).collect(),
                    ..Default::default()
                },
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component bar:
                component: ftd.column
                boolean $open-bar: true

                --- ftd.text: Hello Bar


                -- component foo:
                component: ftd.column
                boolean $open: true

                --- ftd.column:
                id: foo-id

                --- ftd.column:

                --- ftd.text: Click here!
                $event-click$: toggle $open

                --- ftd.text: Hello

                --- container: foo-id

                --- bar:
                if: $open


                -- foo:
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn if_on_var_integer() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Integer(ftd::Text {
                text: markdown_line("20"),
                common: ftd::Common {
                    reference: Some(s("foo/bar#bar")),
                    ..Default::default()
                },
                ..Default::default()
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- $foo: false

                -- $bar: 10

                -- $bar: 20
                if: not $foo

                -- ftd.integer:
                value: $bar

                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn if_on_var_text() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: markdown_line("other-foo says hello"),
            line: true,
            common: ftd::Common {
                reference: Some(s("foo/bar#bar")),
                ..Default::default()
            },
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- $foo: false

                -- $other-foo: true

                -- $bar: hello

                -- $bar: foo says hello
                if: not $foo

                -- $bar: other-foo says hello
                if: $other-foo

                -- ftd.text: $bar

                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn cursor_pointer() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: markdown_line("hello"),
            line: true,
            common: ftd::Common {
                cursor: Some(s("pointer")),
                ..Default::default()
            },
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- ftd.text: hello
                cursor: pointer

                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn comments() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown("hello2"),
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown("/hello3"),
            line: false,
            common: ftd::Common {
                color: Some(ftd::Color {
                    r: 255,
                    g: 0,
                    b: 0,
                    alpha: 1.0,
                }),
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![ftd::Element::Text(ftd::Text {
                    text: ftd::markdown_line("hello5"),
                    line: true,
                    common: ftd::Common {
                        color: Some(ftd::Color {
                            r: 0,
                            g: 128,
                            b: 0,
                            alpha: 1.0,
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                })],
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![ftd::Element::Text(ftd::Text {
                    text: ftd::markdown("/foo says hello"),
                    ..Default::default()
                })],
                ..Default::default()
            },
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                r"
                /-- ftd.text:
                cursor: pointer

                hello1

                -- ftd.text:
                /color: red

                hello2

                -- ftd.text:
                color: red

                \/hello3

                -- ftd.row:

                /--- ftd.text: hello4

                --- ftd.text: hello5
                color: green
                /padding-left: 20

                -- component foo:
                component: ftd.row
                /color: red

                --- ftd.text:

                \/foo says hello

                /--- ftd.text: foo says hello again

                -- foo:

                /-- foo:
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn component_declaration_anywhere_2() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![
                                    ftd::Element::Text(ftd::Text {
                                        text: ftd::markdown_line("Bar says hello"),
                                        line: true,
                                        common: ftd::Common {
                                            reference: Some(s("@name@0,0")),
                                            ..Default::default()
                                        },
                                        ..Default::default()
                                    }),
                                    ftd::Element::Text(ftd::Text {
                                        text: ftd::markdown_line("Hello"),
                                        line: true,
                                        common: ftd::Common {
                                            reference: Some(s("foo/bar#greeting")),
                                            ..Default::default()
                                        },
                                        ..Default::default()
                                    }),
                                ],
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(
                                    s("name@0,0"),
                                    s("Bar says hello"),
                                )])
                                .collect(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("foo says hello"),
                            line: true,
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Hello"),
                            line: true,
                            common: ftd::Common {
                                reference: Some(s("foo/bar#greeting")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                ..Default::default()
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- foo:

                -- component foo:
                component: ftd.column

                --- bar: Bar says hello

                --- ftd.text: foo says hello

                --- ftd.text: $greeting

                -- $greeting: Hello

                -- component bar:
                component: ftd.column
                caption $name:

                --- ftd.text: $name

                --- ftd.text: $greeting
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn action_increment_decrement_condition() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Integer(ftd::Text {
                text: ftd::markdown_line("0"),
                common: ftd::Common {
                    reference: Some(s("foo/bar#count")),
                    ..Default::default()
                },
                ..Default::default()
            }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("Hello on 8"),
            line: true,
            common: ftd::Common {
                condition: Some(ftd::Condition {
                    variable: s("foo/bar#count"),
                    value: s("8"),
                }),
                is_not_visible: true,
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("increment counter"),
            line: true,
            common: ftd::Common {
                events: vec![ftd::Event {
                    name: s("onclick"),
                    action: ftd::Action {
                        action: s("increment"),
                        target: s("foo/bar#count"),
                        parameters: Default::default(),
                    },
                }],
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("decrement counter"),
            line: true,
            common: ftd::Common {
                events: vec![ftd::Event {
                    name: s("onclick"),
                    action: ftd::Action {
                        action: s("decrement"),
                        target: s("foo/bar#count"),
                        parameters: Default::default(),
                    },
                }],
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("increment counter"),
            line: true,
            common: ftd::Common {
                events: vec![ftd::Event {
                    name: s("onclick"),
                    action: ftd::Action {
                        action: s("increment"),
                        target: s("foo/bar#count"),
                        parameters: std::array::IntoIter::new([(s("by"), vec![s("2")])]).collect(),
                    },
                }],
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("increment counter by 2 clamp 2 10"),
            line: true,
            common: ftd::Common {
                events: vec![ftd::Event {
                    name: s("onclick"),
                    action: ftd::Action {
                        action: s("increment"),
                        target: s("foo/bar#count"),
                        parameters: std::array::IntoIter::new([
                            (s("by"), vec![s("2")]),
                            (s("clamp"), vec![s("2"), s("10")]),
                        ])
                        .collect(),
                    },
                }],
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("decrement count clamp 2 10"),
            line: true,
            common: ftd::Common {
                events: vec![ftd::Event {
                    name: s("onclick"),
                    action: ftd::Action {
                        action: s("decrement"),
                        target: s("foo/bar#count"),
                        parameters: std::array::IntoIter::new([(
                            s("clamp"),
                            vec![s("2"), s("10")],
                        )])
                        .collect(),
                    },
                }],
                ..Default::default()
            },
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- $count: 0

                -- ftd.integer:
                value: $count

                -- ftd.text: Hello on 8
                if: $count == 8

                -- ftd.text: increment counter
                $event-click$: increment $count

                -- ftd.text: decrement counter
                $event-click$: decrement $count

                -- ftd.text: increment counter
                $event-click$: increment $count by 2

                -- ftd.text: increment counter by 2 clamp 2 10
                $event-click$: increment $count by 2 clamp 2 10

                -- ftd.text: decrement count clamp 2 10
                $event-click$: decrement $count clamp 2 10
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn action_increment_decrement_local_variable() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Integer(ftd::Text {
                            text: ftd::markdown_line("0"),
                            common: ftd::Common {
                                reference: Some(s("@count@0")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("increment counter"),
                            line: true,
                            common: ftd::Common {
                                events: vec![ftd::Event {
                                    name: s("onclick"),
                                    action: ftd::Action {
                                        action: s("increment"),
                                        target: s("@count@0"),
                                        parameters: std::array::IntoIter::new([(
                                            s("by"),
                                            vec![s("3")],
                                        )])
                                        .collect(),
                                    },
                                }],
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("decrement counter"),
                            line: true,
                            common: ftd::Common {
                                events: vec![ftd::Event {
                                    name: s("onclick"),
                                    action: ftd::Action {
                                        action: s("decrement"),
                                        target: s("@count@0"),
                                        parameters: std::array::IntoIter::new([(
                                            s("by"),
                                            vec![s("2")],
                                        )])
                                        .collect(),
                                    },
                                }],
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([
                        (s("by@0"), s("3")),
                        (s("count@0"), s("0")),
                    ])
                    .collect(),
                    ..Default::default()
                },
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- $decrement-by: 2

                -- component foo:
                component: ftd.column
                integer $by: 4
                integer $count: 0

                --- ftd.integer:
                value: $count

                --- ftd.text: increment counter
                $event-click$: increment $count by $by

                --- ftd.text: decrement counter
                $event-click$: decrement $count by $decrement-by

                -- foo:
                by: 3

                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn nested_component() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Row(ftd::Row {
            common: ftd::Common {
                locals: std::array::IntoIter::new([(s("cta@0"), s("CTA says Hello"))]).collect(),
                ..Default::default()
            },
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- secondary-button: CTA says Hello

                -- component secondary-button:
                component: secondary-button-1
                caption $cta:
                cta: $cta


                -- component secondary-button-1:
                component: ftd.row
                caption $cta:

                --- ftd.text: $cta
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn action_increment_decrement_on_component() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Image(ftd::Image {
                src: s("https://www.liveabout.com/thmb/YCJmu1khSJo8kMYM090QCd9W78U=/1250x0/filters:no_upscale():max_bytes(150000):strip_icc():format(webp)/powerpuff_girls-56a00bc45f9b58eba4aea61d.jpg"),
                common: ftd::Common {
                    condition: Some(
                        ftd::Condition {
                            variable: s("foo/bar#count"),
                            value: s("0"),
                        },
                    ),
                    is_not_visible: false,
                    events: vec![
                        ftd::Event {
                            name: s("onclick"),
                            action: ftd::Action {
                                action: s("increment"),
                                target: s("foo/bar#count"),
                                parameters: std::array::IntoIter::new([(s("clamp"), vec![s("0"), s("1")])])
                                    .collect(),
                            },
                        },
                    ],
                    locals: std::array::IntoIter::new([
                        (s("idx@0"), s("0")),
                        (s("src@0"), s("https://www.liveabout.com/thmb/YCJmu1khSJo8kMYM090QCd9W78U=/1250x0/filters:no_upscale():max_bytes(150000):strip_icc():format(webp)/powerpuff_girls-56a00bc45f9b58eba4aea61d.jpg"))
                    ]).collect(),
                    reference: Some(s("@src@0")),
                    ..Default::default()
                },
                ..Default::default()
            }));

        main.container
            .children
            .push(ftd::Element::Image(ftd::Image {
                src: s("https://upload.wikimedia.org/wikipedia/en/d/d4/Mickey_Mouse.png"),
                common: ftd::Common {
                    condition: Some(ftd::Condition {
                        variable: s("foo/bar#count"),
                        value: s("1"),
                    }),
                    is_not_visible: true,
                    events: vec![ftd::Event {
                        name: s("onclick"),
                        action: ftd::Action {
                            action: s("increment"),
                            target: s("foo/bar#count"),
                            parameters: std::array::IntoIter::new([(
                                s("clamp"),
                                vec![s("0"), s("1")],
                            )])
                            .collect(),
                        },
                    }],
                    locals: std::array::IntoIter::new([
                        (s("idx@1"), s("1")),
                        (
                            s("src@1"),
                            s("https://upload.wikimedia.org/wikipedia/en/d/d4/Mickey_Mouse.png"),
                        ),
                    ])
                    .collect(),
                    reference: Some(s("@src@1")),
                    ..Default::default()
                },
                ..Default::default()
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- $count: 0

                -- component slide:
                component: ftd.image
                string $src:
                integer $idx:
                src: $src
                if: $count == $idx
                $event-click$: increment $count clamp 0 1

                -- slide:
                src: https://www.liveabout.com/thmb/YCJmu1khSJo8kMYM090QCd9W78U=/1250x0/filters:no_upscale():max_bytes(150000):strip_icc():format(webp)/powerpuff_girls-56a00bc45f9b58eba4aea61d.jpg
                idx: 0

                -- slide:
                src: https://upload.wikimedia.org/wikipedia/en/d/d4/Mickey_Mouse.png
                idx: 1
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn loop_on_list_string() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Arpita"),
                            line: true,
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Ayushi"),
                            line: true,
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("AmitU"),
                            line: true,
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                ..Default::default()
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component foo:
                component: ftd.column
                string list $bar:

                --- ftd.text: $obj
                $loop$: $bar as $obj

                -- string list $names:

                -- $names: Arpita

                -- $names: Ayushi

                -- $names: AmitU

                -- foo:
                bar: $names
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn open_container_with_parent_id() {
        let mut main = super::default_column();
        let beverage_external_children = vec![ftd::Element::Column(ftd::Column {
            container: ftd::Container {
                children: vec![
                    ftd::Element::Column(ftd::Column {
                        container: ftd::Container {
                            children: vec![
                                ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("Water"),
                                    line: true,
                                    common: ftd::Common {
                                        events: vec![ftd::Event {
                                            name: s("onclick"),
                                            action: ftd::Action {
                                                action: s("toggle"),
                                                target: s("@visible@0,0,0"),
                                                ..Default::default()
                                            },
                                        }],
                                        reference: Some(s("@name@0,0,0")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }),
                                ftd::Element::Column(ftd::Column {
                                    common: ftd::Common {
                                        condition: Some(ftd::Condition {
                                            variable: s("@visible@0,0,0"),
                                            value: s("true"),
                                        }),
                                        data_id: Some(s("some-child")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }),
                            ],
                            external_children: Some((s("some-child"), vec![vec![1]], vec![])),
                            open: (None, Some(s("some-child"))),
                            ..Default::default()
                        },
                        common: ftd::Common {
                            locals: std::array::IntoIter::new([
                                (s("name@0,0,0"), s("Water")),
                                (s("visible@0,0,0"), s("true")),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    }),
                    ftd::Element::Column(ftd::Column {
                        container: ftd::Container {
                            children: vec![
                                ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("Juice"),
                                    line: true,
                                    common: ftd::Common {
                                        events: vec![ftd::Event {
                                            name: s("onclick"),
                                            action: ftd::Action {
                                                action: s("toggle"),
                                                target: s("@visible@0,0,1"),
                                                ..Default::default()
                                            },
                                        }],
                                        reference: Some(s("@name@0,0,1")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }),
                                ftd::Element::Column(ftd::Column {
                                    common: ftd::Common {
                                        condition: Some(ftd::Condition {
                                            variable: s("@visible@0,0,1"),
                                            value: s("true"),
                                        }),
                                        data_id: Some(s("some-child")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }),
                            ],
                            external_children: Some((
                                s("some-child"),
                                vec![vec![1]],
                                vec![ftd::Element::Column(ftd::Column {
                                    container: ftd::Container {
                                        children: vec![ftd::Element::Column(ftd::Column {
                                            container: ftd::Container {
                                                children: vec![
                                                    ftd::Element::Text(ftd::Text {
                                                        text: ftd::markdown_line("Mango Juice"),
                                                        line: true,
                                                        common: ftd::Common {
                                                            events: vec![ftd::Event {
                                                                name: s("onclick"),
                                                                action: ftd::Action {
                                                                    action: s("toggle"),
                                                                    target: s("@visible@0,0,1,0"),
                                                                    ..Default::default()
                                                                },
                                                            }],
                                                            reference: Some(s("@name@0,0,1,0")),
                                                            ..Default::default()
                                                        },
                                                        ..Default::default()
                                                    }),
                                                    ftd::Element::Column(ftd::Column {
                                                        common: ftd::Common {
                                                            condition: Some(ftd::Condition {
                                                                variable: s("@visible@0,0,1,0"),
                                                                value: s("true"),
                                                            }),
                                                            data_id: Some(s("some-child")),
                                                            ..Default::default()
                                                        },
                                                        ..Default::default()
                                                    }),
                                                ],
                                                external_children: Some((
                                                    s("some-child"),
                                                    vec![vec![1]],
                                                    vec![],
                                                )),
                                                open: (None, Some(s("some-child"))),
                                                ..Default::default()
                                            },
                                            common: ftd::Common {
                                                locals: std::array::IntoIter::new([
                                                    (s("name@0,0,1,0"), s("Mango Juice")),
                                                    (s("visible@0,0,1,0"), s("true")),
                                                ])
                                                .collect(),
                                                ..Default::default()
                                            },
                                        })],
                                        wrap: true,
                                        ..Default::default()
                                    },
                                    common: ftd::Common {
                                        width: Some(ftd::Length::Fill),
                                        height: Some(ftd::Length::Fill),
                                        position: ftd::Position::Center,
                                        ..Default::default()
                                    },
                                })],
                            )),
                            open: (None, Some(s("some-child"))),
                            ..Default::default()
                        },
                        common: ftd::Common {
                            locals: std::array::IntoIter::new([
                                (s("name@0,0,1"), s("Juice")),
                                (s("visible@0,0,1"), s("true")),
                            ])
                            .collect(),
                            ..Default::default()
                        },
                    }),
                ],
                wrap: true,
                ..Default::default()
            },
            common: ftd::Common {
                width: Some(ftd::Length::Fill),
                height: Some(ftd::Length::Fill),
                position: ftd::Position::Center,
                ..Default::default()
            },
        })];

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Column(ftd::Column {
                        container: ftd::Container {
                            children: vec![
                                ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("Beverage"),
                                    line: true,
                                    common: ftd::Common {
                                        events: vec![ftd::Event {
                                            name: s("onclick"),
                                            action: ftd::Action {
                                                action: s("toggle"),
                                                target: s("@visible@0,0"),
                                                ..Default::default()
                                            },
                                        }],
                                        reference: Some(s("@name@0,0")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }),
                                ftd::Element::Column(ftd::Column {
                                    common: ftd::Common {
                                        condition: Some(ftd::Condition {
                                            variable: s("@visible@0,0"),
                                            value: s("true"),
                                        }),
                                        data_id: Some(s("some-child")),
                                        id: Some(s("beverage:some-child")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }),
                            ],
                            external_children: Some((
                                s("some-child"),
                                vec![vec![1]],
                                beverage_external_children,
                            )),
                            open: (None, Some(s("some-child"))),
                            ..Default::default()
                        },
                        common: ftd::Common {
                            locals: std::array::IntoIter::new([
                                (s("name@0,0"), s("Beverage")),
                                (s("visible@0,0"), s("true")),
                            ])
                            .collect(),
                            data_id: Some(s("beverage")),
                            id: Some(s("beverage")),
                            ..Default::default()
                        },
                    })],
                    ..Default::default()
                },
                ..Default::default()
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
            -- component display-item1:
            component: ftd.column
            string $name:
            open: some-child
            boolean $visible: true

            --- ftd.text: $name
            $event-click$: toggle $visible

            --- ftd.column:
            if: $visible
            id: some-child

            -- ftd.column:

            -- display-item1:
            name: Beverage
            id: beverage


            -- display-item1:
            name: Water


            -- container: beverage


            -- display-item1:
            name: Juice


            -- display-item1:
            name: Mango Juice
            "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn text_check() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("$hello"),
                            line: true,
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("hello"),
                            line: true,
                            common: ftd::Common {
                                reference: Some(s("@hello2@0")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("hello"),
                            line: true,
                            common: ftd::Common {
                                reference: Some(s("foo/bar#hello")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("hello"),
                            line: true,
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([(s("hello2@0"), s("hello"))]).collect(),
                    ..Default::default()
                },
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                r"
                -- $hello: hello

                -- component foo:
                component: ftd.column
                string $hello2:

                --- ftd.text: \$hello

                --- ftd.text: $hello2

                --- ftd.text: $hello

                --- ftd.text: hello

                -- foo:
                hello2: $hello
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn caption() {
        let mut main = super::default_column();

        main.container
            .children
            .push(ftd::Element::Integer(ftd::Text {
                text: ftd::markdown_line("32"),
                ..Default::default()
            }));

        main.container
            .children
            .push(ftd::Element::Boolean(ftd::Text {
                text: ftd::markdown_line("true"),
                ..Default::default()
            }));

        main.container
            .children
            .push(ftd::Element::Decimal(ftd::Text {
                text: ftd::markdown_line("0.06"),
                ..Default::default()
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- ftd.integer: 32

                -- ftd.boolean: true

                -- ftd.decimal: 0.06
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn heading_id() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Heading 00"),
                            line: true,
                            common: ftd::Common {
                                region: Some(ftd::Region::Title),
                                reference: Some(s("@title@0")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown("Heading 00 body"),
                            common: ftd::Common {
                                id: Some(s("one:markdown-id")),
                                data_id: Some(s("markdown-id")),
                                locals: std::array::IntoIter::new([(
                                    s("body@0,1"),
                                    s("Heading 00 body"),
                                )])
                                .collect(),
                                reference: Some(s("@body@0,1")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    region: Some(ftd::Region::H0),
                    id: Some(s("one")),
                    data_id: Some(s("one")),
                    locals: std::array::IntoIter::new([
                        (s("body@0"), s("Heading 00 body")),
                        (s("title@0"), s("Heading 00")),
                    ])
                    .collect(),
                    ..Default::default()
                },
            }));

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Heading 01"),
                            line: true,
                            common: ftd::Common {
                                region: Some(ftd::Region::Title),
                                reference: Some(s("@title@1")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown("Heading 01 body"),
                            common: ftd::Common {
                                data_id: Some(s("markdown-id")),
                                locals: std::array::IntoIter::new([(
                                    s("body@1,1"),
                                    s("Heading 01 body"),
                                )])
                                .collect(),
                                reference: Some(s("@body@1,1")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    region: Some(ftd::Region::H0),
                    id: Some(s("heading-01")),
                    locals: std::array::IntoIter::new([
                        (s("body@1"), s("Heading 01 body")),
                        (s("title@1"), s("Heading 01")),
                    ])
                    .collect(),
                    ..Default::default()
                },
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- h0: Heading 00
                id: one

                Heading 00 body

                -- h0: Heading 01

                Heading 01 body

                -- component h0:
                component: ftd.column
                caption $title:
                optional body $body:
                region: h0

                --- ftd.text:
                text: $title
                region: title

                --- markdown:
                if: $body is not null
                body: $body
                id: markdown-id

                -- component markdown:
                component: ftd.text
                body $body:
                text: $body
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn new_id() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("hello"),
                        line: true,
                        common: ftd::Common {
                            data_id: Some(s("hello")),
                            ..Default::default()
                        },
                        ..Default::default()
                    })],
                    ..Default::default()
                },
                ..Default::default()
            }));

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("hello"),
                        line: true,
                        common: ftd::Common {
                            data_id: Some(s("hello")),
                            id: Some(s("asd:hello")),
                            ..Default::default()
                        },
                        ..Default::default()
                    })],
                    ..Default::default()
                },
                common: ftd::Common {
                    data_id: Some(s("asd")),
                    id: Some(s("asd")),
                    ..Default::default()
                },
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
            --  component foo:
            component: ftd.column

            --- ftd.text: hello
            id: hello

            -- foo:

            -- foo:
            id: asd
            "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn list_is_empty_check() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("Hello people"),
            line: true,
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Null);

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Null,
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Hello empty list"),
                            line: true,
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                ..Default::default()
            }));

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Hello list"),
                            line: true,
                            ..Default::default()
                        }),
                        ftd::Element::Null,
                    ],
                    ..Default::default()
                },
                ..Default::default()
            }));
        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- string list $people:

                -- $people: Ayushi

                -- $people: Arpita

                -- ftd.text: Hello people
                if: $people is not empty

                -- ftd.text: Hello nobody
                if: $people is empty


                -- string list $empty-list:


                -- component foo:
                component: ftd.column
                string list $string-list:

                --- ftd.text: Hello list
                if: $string-list is not empty

                --- ftd.text: Hello empty list
                if: $string-list is empty

                -- foo:
                string-list: $empty-list

                -- foo:
                string-list: $people
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn parent_with_unsatified_condition() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Null);
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("Hello"),
                        line: true,
                        ..Default::default()
                    })],
                    ..Default::default()
                },
                common: ftd::Common {
                    is_not_visible: true,
                    ..Default::default()
                },
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- string list $empty-list:

                -- ftd.column:
                if: $empty-list is not empty

                --- ftd.text: Hello

                -- foo:

                -- component foo:
                component: ftd.column
                if: $empty-list is not empty

                --- ftd.text: Hello
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn open_container_id_with_children() {
        let mut external_children = super::default_column();
        external_children
            .container
            .children
            .push(ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("hello"),
                line: true,
                ..Default::default()
            }));
        external_children
            .container
            .children
            .push(ftd::Element::Text(ftd::Text {
                text: ftd::markdown_line("world"),
                line: true,
                ..Default::default()
            }));

        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![ftd::Element::Column(ftd::Column {
                        common: ftd::Common {
                            id: Some(s("foo-id:some-id")),
                            data_id: Some(s("some-id")),
                            ..Default::default()
                        },
                        ..Default::default()
                    })],
                    external_children: Some((
                        s("some-id"),
                        vec![vec![0]],
                        vec![ftd::Element::Column(external_children)],
                    )),
                    open: (None, Some(s("some-id"))),
                    ..Default::default()
                },
                common: ftd::Common {
                    id: Some(s("foo-id")),
                    data_id: Some(s("foo-id")),
                    ..Default::default()
                },
            }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("Outside"),
            line: true,
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- foo:
                id: foo-id

                --- ftd.text: hello

                --- ftd.text: world

                -- ftd.text: Outside


                -- component foo:
                component: ftd.column
                open: some-id

                --- ftd.column:
                id: some-id
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn loop_record_list() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("commit message 1"),
                                    line: true,
                                    common: ftd::Common {
                                        reference: Some(s("@commit.message")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("commit message 2"),
                                    line: true,
                                    common: ftd::Common {
                                        reference: Some(s("@commit.message")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("file filename 1"),
                                    line: true,
                                    common: ftd::Common {
                                        reference: Some(s("@file.filename")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("file filename 2"),
                                    line: true,
                                    common: ftd::Common {
                                        reference: Some(s("@file.filename")),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                ..Default::default()
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- record commit:
                string message:

                -- record file:
                string filename:

                -- record changes:
                commit list commits:
                file list files:


                -- commit list $commit-list:

                -- $commit-list:
                message: commit message 1

                -- $commit-list:
                message: commit message 2


                -- file list $file-list:

                -- $file-list:
                filename: file filename 1

                -- $file-list:
                filename: file filename 2


                -- changes $rec-changes:
                commits: $commit-list
                files: $file-list

                -- display:
                changes: $rec-changes




                -- component display:
                component: ftd.column
                changes $changes:

                --- display-commit:
                $loop$: $changes.commits as $obj
                commit: $obj

                --- display-file:
                $loop$: $changes.files as $obj
                file: $obj


                -- component display-commit:
                component: ftd.column
                commit $commit:

                --- ftd.text: $commit.message


                -- component display-file:
                component: ftd.column
                file $file:

                --- ftd.text: $file.filename
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn scene_children_with_default_position() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Scene(ftd::Scene {
                container: ftd::Container {
                    children: vec![ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("Hello"),
                        line: true,
                        common: ftd::Common {
                            top: Some(0),
                            left: Some(0),
                            ..Default::default()
                        },
                        ..Default::default()
                    }), ftd::Element::Text(ftd::Text {
                        text: ftd::markdown_line("World"),
                        line: true,
                        common: ftd::Common {
                            top: Some(10),
                            right: Some(30),
                            scale: Some(1.5),
                            scale_x: Some(-1.0),
                            scale_y: Some(-1.0),
                            rotate: Some(45),
                            position: ftd::Position::Center,
                            ..Default::default()
                        },
                        ..Default::default()
                    })],
                    ..Default::default()
                },
                common: ftd::Common {
                    width: Some(
                        ftd::Length::Px {
                            value: 1000,
                        },
                    ),
                    background_image: Some(
                        s("https://image.shutterstock.com/z/stock-&lt;!&ndash;&ndash;&gt;vector-vector-illustration-of-a-beautiful-summer-landscape-143054302.jpg"),
                    ),
                    ..Default::default()
                }
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- ftd.scene:
                background-image: https://image.shutterstock.com/z/stock-&lt;!&ndash;&ndash;&gt;vector-vector-illustration-of-a-beautiful-summer-landscape-143054302.jpg
                width: 1000

                --- ftd.text: Hello

                --- foo:
                top: 10
                right: 30
                align: center
                scale: 1.5
                rotate: 45
                scale-x: -1
                scale-y: -1

                -- component foo:
                component: ftd.text
                text: World
                "
            ),
            &ftd::p2::TestLibrary {},
        )
            .expect("found error");

        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn event_set() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("Start..."),
            line: true,
            common: ftd::Common {
                condition: Some(ftd::Condition {
                    variable: s("foo/bar#current"),
                    value: s("some value"),
                }),
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("some value"),
            line: true,
            common: ftd::Common {
                reference: Some(s("foo/bar#current")),
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("change message"),
            line: true,
            common: ftd::Common {
                events: vec![ftd::Event {
                    name: s("onclick"),
                    action: ftd::Action {
                        action: s("set-value"),
                        target: s("foo/bar#current"),
                        parameters: std::array::IntoIter::new([(
                            s("value"),
                            vec![s("hello world"), s("string")],
                        )])
                        .collect(),
                    },
                }],
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("change message again"),
            line: true,
            common: ftd::Common {
                events: vec![ftd::Event {
                    name: s("onclick"),
                    action: ftd::Action {
                        action: s("set-value"),
                        target: s("foo/bar#current"),
                        parameters: std::array::IntoIter::new([(
                            s("value"),
                            vec![s("good bye"), s("string")],
                        )])
                        .collect(),
                    },
                }],
                ..Default::default()
            },
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- $current: some value

                -- ftd.text: Start...
                if: $current == some value

                -- ftd.text: $current

                -- ftd.text: change message
                $event-click$: $current = hello world

                -- $msg: good bye

                -- ftd.text: change message again
                $event-click$: $current = $msg
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn absolute_positioning() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello world"),
            line: true,
            common: ftd::Common {
                anchor: Some(ftd::Anchor::Parent),
                right: Some(0),
                top: Some(100),
                ..Default::default()
            },
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- ftd.text: hello world
                anchor: parent
                right: 0
                top: 100
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn inherit_check() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello"),
            line: true,
            size: Some(50),
            common: ftd::Common {
                locals: std::array::IntoIter::new([(s("size@0"), s("50"))]).collect(),
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello"),
            line: true,
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component foo: hello
                component: ftd.text
                inherit $size:

                -- foo:
                size: 50

                -- foo:
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn inner_container_check() {
        let mut main = super::default_column();
        let col = ftd::Element::Column(ftd::Column {
            container: ftd::Container {
                children: vec![ftd::Element::Column(ftd::Column {
                    container: ftd::Container {
                        children: vec![
                            ftd::Element::Image(ftd::Image {
                                src: s("https://www.nilinswap.com/static/img/dp.jpeg"),
                                ..Default::default()
                            }),
                            ftd::Element::Text(ftd::Text {
                                text: ftd::markdown_line("Swapnil Sharma"),
                                line: true,
                                ..Default::default()
                            }),
                        ],
                        ..Default::default()
                    },
                    ..Default::default()
                })],
                ..Default::default()
            },
            ..Default::default()
        });
        main.container.children.push(col.clone());
        main.container.children.push(col);

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- ftd.column:

                --- ftd.column:

                --- ftd.image:
                src: https://www.nilinswap.com/static/img/dp.jpeg

                --- ftd.text: Swapnil Sharma


                -- component foo:
                component: ftd.column

                --- ftd.column:

                --- ftd.image:
                src: https://www.nilinswap.com/static/img/dp.jpeg

                --- ftd.text: Swapnil Sharma

                -- foo:
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn mouse_in() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("Hello World"),
            line: true,
            common: ftd::Common {
                locals: std::array::IntoIter::new([(s("MOUSE-IN@0"), s("false"))]).collect(),
                conditional_attribute: std::array::IntoIter::new([(
                    s("color"),
                    ftd::ConditionalAttribute {
                        attribute_type: ftd::AttributeType::Style,
                        conditions_with_value: vec![(
                            ftd::Condition {
                                variable: s("@MOUSE-IN@0"),
                                value: s("true"),
                            },
                            ftd::ConditionalValue {
                                value: s("rgba(255,0,0,1)"),
                                important: false,
                            },
                        )],
                        default: None,
                    },
                )])
                .collect(),
                events: vec![
                    ftd::Event {
                        name: s("onmouseenter"),
                        action: ftd::Action {
                            action: s("set-value"),
                            target: s("@MOUSE-IN@0"),
                            parameters: std::array::IntoIter::new([(
                                s("value"),
                                vec![s("true"), s("boolean")],
                            )])
                            .collect(),
                        },
                    },
                    ftd::Event {
                        name: s("onmouseleave"),
                        action: ftd::Action {
                            action: s("set-value"),
                            target: s("@MOUSE-IN@0"),
                            parameters: std::array::IntoIter::new([(
                                s("value"),
                                vec![s("false"), s("boolean")],
                            )])
                            .collect(),
                        },
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component foo:
                component: ftd.text
                text: Hello World
                color if $MOUSE-IN: red

                -- foo:
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn event_stop_propagation() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Hello"),
                            line: true,
                            common: ftd::Common {
                                condition: Some(ftd::Condition {
                                    variable: s("@open@0"),
                                    value: s("true"),
                                }),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Column(ftd::Column {
                            container: ftd::Container {
                                children: vec![ftd::Element::Text(ftd::Text {
                                    text: ftd::markdown_line("Hello Again"),
                                    line: true,
                                    common: ftd::Common {
                                        condition: Some(ftd::Condition {
                                            variable: s("@open@0,1"),
                                            value: s("true"),
                                        }),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                })],
                                ..Default::default()
                            },
                            common: ftd::Common {
                                locals: std::array::IntoIter::new([(s("open@0,1"), s("true"))])
                                    .collect(),
                                events: vec![
                                    ftd::Event {
                                        name: s("onclick"),
                                        action: ftd::Action {
                                            action: s("toggle"),
                                            target: s("@open@0,1"),
                                            parameters: Default::default(),
                                        },
                                    },
                                    ftd::Event {
                                        name: s("onclick"),
                                        action: ftd::Action {
                                            action: s("stop-propagation"),
                                            target: s(""),
                                            parameters: Default::default(),
                                        },
                                    },
                                ],
                                ..Default::default()
                            },
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([(s("open@0"), s("true"))]).collect(),
                    events: vec![ftd::Event {
                        name: s("onclick"),
                        action: ftd::Action {
                            action: s("toggle"),
                            target: s("@open@0"),
                            parameters: Default::default(),
                        },
                    }],
                    ..Default::default()
                },
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- foo:

                -- component foo:
                component: ftd.column
                boolean $open: true
                $event-click$: toggle $open

                --- ftd.text: Hello
                if: $open

                --- bar:


                -- component bar:
                component: ftd.column
                boolean $open: true
                $event-click$: toggle $open
                $event-click$: stop-propagation

                --- ftd.text: Hello Again
                if: $open

                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn new_syntax() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![ftd::Element::Integer(ftd::Text {
                    text: ftd::markdown_line("20"),
                    common: ftd::Common {
                        conditional_attribute: std::array::IntoIter::new([(
                            s("color"),
                            ftd::ConditionalAttribute {
                                attribute_type: ftd::AttributeType::Style,
                                conditions_with_value: vec![
                                    (
                                        ftd::Condition {
                                            variable: s("@b@0"),
                                            value: s("true"),
                                        },
                                        ftd::ConditionalValue {
                                            value: s("rgba(0,0,0,1)"),
                                            important: false,
                                        },
                                    ),
                                    (
                                        ftd::Condition {
                                            variable: s("@a@0"),
                                            value: s("30"),
                                        },
                                        ftd::ConditionalValue {
                                            value: s("rgba(255,0,0,1)"),
                                            important: false,
                                        },
                                    ),
                                ],
                                default: None,
                            },
                        )])
                        .collect(),
                        reference: Some(s("@a@0")),
                        ..Default::default()
                    },
                    ..Default::default()
                })],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([(s("a@0"), s("20")), (s("b@0"), s("false"))])
                    .collect(),
                events: vec![
                    ftd::Event {
                        name: s("onclick"),
                        action: ftd::Action {
                            action: s("toggle"),
                            target: s("@b@0"),
                            parameters: Default::default(),
                        },
                    },
                    ftd::Event {
                        name: s("onclick"),
                        action: ftd::Action {
                            action: s("increment"),
                            target: s("@a@0"),
                            parameters: std::array::IntoIter::new([(s("by"), vec![s("2")])])
                                .collect(),
                        },
                    },
                ],
                ..Default::default()
            },
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component foo:
                component: ftd.row
                integer $a:
                boolean $b: false
                $event-click$: toggle $b
                $event-click$: increment $a by 2

                --- ftd.integer:
                value: $a
                color if $b: black
                color if $a == 30: red

                -- foo:
                a: 20
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn condition_check() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![ftd::Element::Column(ftd::Column {
                    container: ftd::Container {
                        children: vec![ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("Hello"),
                            line: true,
                            common: ftd::Common {
                                condition: Some(ftd::Condition {
                                    variable: s("@b@0,0"),
                                    value: s("true"),
                                }),
                                is_not_visible: true,
                                ..Default::default()
                            },
                            ..Default::default()
                        })],
                        ..Default::default()
                    },
                    common: ftd::Common {
                        locals: std::array::IntoIter::new([
                            (s("a@0,0"), s("true")),
                            (s("b@0,0"), s("false")),
                        ])
                        .collect(),
                        condition: Some(ftd::Condition {
                            variable: s("@b@0"),
                            value: s("true"),
                        }),
                        ..Default::default()
                    },
                })],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([(s("b@0"), s("true"))]).collect(),
                ..Default::default()
            },
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- $present: true

                -- component bar:
                component: ftd.column
                boolean $a: true
                if: $a
                boolean $b: false

                --- ftd.text: Hello
                if: $b

                -- component foo:
                component: ftd.row
                boolean $b: true

                --- bar:
                if: $b

                -- foo:
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn external_variable() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Integer(ftd::Text {
                            text: ftd::markdown_line("20"),
                            common: ftd::Common {
                                conditional_attribute: std::array::IntoIter::new([(
                                    s("color"),
                                    ftd::ConditionalAttribute {
                                        attribute_type: ftd::AttributeType::Style,
                                        conditions_with_value: vec![(
                                            ftd::Condition {
                                                variable: s("@b@0"),
                                                value: s("true"),
                                            },
                                            ftd::ConditionalValue {
                                                value: s("rgba(0,0,0,1)"),
                                                important: false,
                                            },
                                        )],
                                        default: None,
                                    },
                                )])
                                .collect(),
                                reference: Some(s("@a@0")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("whatever"),
                            line: true,
                            common: ftd::Common {
                                reference: Some(s("@some-text@0")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([
                        (s("a@0"), s("20")),
                        (s("b@0"), s("false")),
                        (s("some-text@0"), s("whatever")),
                    ])
                    .collect(),
                    events: vec![
                        ftd::Event {
                            name: s("onclick"),
                            action: ftd::Action {
                                action: s("toggle"),
                                target: s("@b@0"),
                                parameters: Default::default(),
                            },
                        },
                        ftd::Event {
                            name: s("onclick"),
                            action: ftd::Action {
                                action: s("increment"),
                                target: s("@a@0"),
                                parameters: Default::default(),
                            },
                        },
                        ftd::Event {
                            name: s("onclick"),
                            action: ftd::Action {
                                action: s("set-value"),
                                target: s("@some-text@0"),
                                parameters: std::array::IntoIter::new([(
                                    "value".to_string(),
                                    vec!["hello".to_string(), "string".to_string()],
                                )])
                                .collect(),
                            },
                        },
                    ],
                    ..Default::default()
                },
            }));

        main.container.children.push(ftd::Element::Row(ftd::Row {
            container: ftd::Container {
                children: vec![ftd::Element::Text(ftd::Text {
                    text: ftd::markdown_line("hello"),
                    line: true,
                    common: ftd::Common {
                        conditional_attribute: std::array::IntoIter::new([(
                            s("color"),
                            ftd::ConditionalAttribute {
                                attribute_type: ftd::AttributeType::Style,
                                conditions_with_value: vec![(
                                    ftd::Condition {
                                        variable: s("@foo@1"),
                                        value: s("true"),
                                    },
                                    ftd::ConditionalValue {
                                        value: s("rgba(255,0,0,1)"),
                                        important: false,
                                    },
                                )],
                                default: None,
                            },
                        )])
                        .collect(),
                        ..Default::default()
                    },
                    ..Default::default()
                })],
                ..Default::default()
            },
            common: ftd::Common {
                locals: std::array::IntoIter::new([(s("foo@1"), s("false"))]).collect(),
                events: vec![ftd::Event {
                    name: s("onclick"),
                    action: ftd::Action {
                        action: s("toggle"),
                        target: s("@foo@1"),
                        parameters: Default::default(),
                    },
                }],
                ..Default::default()
            },
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component foo:
                component: ftd.column
                integer $a:
                boolean $b: false
                $event-click$: toggle $b
                $event-click$: increment $a

                --- ftd.integer:
                value: $a
                color if $b: black

                -- $current: hello

                -- foo:
                a: 20
                string $some-text: whatever
                $event-click$: $some-text = $current

                --- ftd.text: $some-text

                -- ftd.row:
                boolean $foo: false
                $event-click$: toggle $foo

                --- ftd.text: hello
                color if $foo: red
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn new_var_syntax() {
        let mut main = super::default_column();
        main.container.children.push(ftd::Element::Text(ftd::Text {
            text: ftd::markdown_line("hello"),
            line: true,
            size: Some(30),
            common: ftd::Common {
                conditional_attribute: std::array::IntoIter::new([(
                    s("color"),
                    ftd::ConditionalAttribute {
                        attribute_type: ftd::AttributeType::Style,
                        conditions_with_value: vec![(
                            ftd::Condition {
                                variable: s("@t@0"),
                                value: s("true"),
                            },
                            ftd::ConditionalValue {
                                value: s("rgba(255,0,0,1)"),
                                important: false,
                            },
                        )],
                        default: None,
                    },
                )])
                .collect(),
                locals: std::array::IntoIter::new([(s("f@0"), s("hello")), (s("t@0"), s("true"))])
                    .collect(),
                reference: Some(s("foo/bar#bar")),
                color: Some(ftd::Color {
                    r: 255,
                    g: 0,
                    b: 0,
                    alpha: 1.0,
                }),
                ..Default::default()
            },
            ..Default::default()
        }));

        main.container
            .children
            .push(ftd::Element::Column(ftd::Column {
                container: ftd::Container {
                    children: vec![
                        ftd::Element::Text(ftd::Text {
                            text: ftd::markdown_line("hello"),
                            line: true,
                            common: ftd::Common {
                                reference: Some(s("@ff@1")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                        ftd::Element::Integer(ftd::Text {
                            text: ftd::markdown_line("20"),
                            common: ftd::Common {
                                reference: Some(s("@i@1")),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    ],
                    ..Default::default()
                },
                common: ftd::Common {
                    locals: std::array::IntoIter::new([
                        (s("ff@1"), s("hello")),
                        (s("i@1"), s("20")),
                    ])
                    .collect(),
                    ..Default::default()
                },
            }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- component col:
                component: ftd.column
                integer $i:
                $ff: hello

                --- ftd.text: $ff

                --- ftd.integer: $i

                -- integer $foo: 20

                -- $foo: 30

                -- $bar: hello

                -- ftd.text: $bar
                boolean $t: true
                $f: hello
                size: $foo
                color if $t: red

                -- col:
                i: 20
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    #[test]
    fn text_block() {
        let mut main = super::default_column();
        main.container
            .children
            .push(ftd::Element::TextBlock(ftd::TextBlock {
                text: ftd::markdown_line("hello"),
                line: true,
                ..Default::default()
            }));

        main.container
            .children
            .push(ftd::Element::TextBlock(ftd::TextBlock {
                text: ftd::markdown_line("hello"),
                line: true,
                ..Default::default()
            }));

        main.container.children.push(ftd::Element::Code(ftd::Code {
            text: ftd::code_with_theme(
                "This is text",
                "txt",
                ftd::render::DEFAULT_THEME,
                "foo/bar",
            )
            .unwrap(),
            ..Default::default()
        }));

        let (_g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- ftd.text-block: hello

                -- component b: hello
                component: ftd.text-block

                -- b:

                -- ftd.code:

                This is text
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        pretty_assertions::assert_eq!(g_col, main);
    }

    /*#[test]
    fn loop_with_tree_structure_1() {
        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- record toc-record:
                title: string
                link: string
                children: list toc-record

                -- component toc-item:
                component: ftd.column
                toc-record $toc:
                padding-left: 10

                --- ftd.text: ref $toc.title
                link: ref $toc.link

                --- toc-item:
                $loop$: $toc.children as $obj
                toc: $obj


                -- toc-record list $toc:

                -- $toc:
                title: ref ab.title
                link: ref ab.link
                children: ref ab.children

                -- toc-record $ab:
                title: ab title
                link: ab link

                -- ab.children $first_ab
                title: aa title
                link: aa link

                --- children:
                title:

                -- ab.children:
                title: aaa title
                link: aaa link



                -- toc-item:
                $loop$: toc as $obj
                toc: $obj
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        // pretty_assertions::assert_eq!(g_bag, bag);
        // pretty_assertions::assert_eq!(g_col, main);
        // --- toc-item:
        //                 $loop$: $toc.children as $t
        //                 toc: $t
    }

    #[test]
    fn loop_with_tree_structure_2() {
        let (g_bag, g_col) = crate::p2::interpreter::interpret(
            "foo/bar",
            indoc::indoc!(
                "
                -- record toc-record:
                title: string
                link: string
                children: list toc-record

                -- component toc-item:
                component: ftd.column
                toc-record $toc:
                padding-left: 10

                --- ftd.text: ref $toc.title
                link: ref $toc.link

                --- toc-item:
                $loop$: $toc.children as $obj
                toc: $obj


                -- toc-record list $toc:
                $processor$: ft.toc

                - fifthtry/ftd/p1
                  `ftd::p1`: A JSON/YML Replacement
                - fifthtry/ftd/language
                  FTD Language
                  - fifthtry/ftd/p1-grammar
                    `ftd::p1` grammar




                -- toc-item:
                $loop$: $toc as $obj
                toc: $obj
                "
            ),
            &ftd::p2::TestLibrary {},
        )
        .expect("found error");
        // pretty_assertions::assert_eq!(g_bag, bag);
        // pretty_assertions::assert_eq!(g_col, main);
        // --- toc-item:
        //                 $loop$: $toc.children as $t
        //                 toc: $t
    }*/
}