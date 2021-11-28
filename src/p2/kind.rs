#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum Kind {
    String {
        caption: bool,
        body: bool,
        default: Option<String>,
    },
    Integer {
        default: Option<String>,
    },
    Decimal {
        default: Option<String>,
    },
    Boolean {
        default: Option<String>,
    },
    Element,
    Elements,
    Message,
    StringMessage, // message that takes a string
    IntMessage,    // message that takes an int
    Record {
        name: String,
    }, // the full name of the record (full document name.record name)
    OrType {
        name: String,
    }, // the full name of the or-type
    Map {
        kind: Box<Kind>,
    }, // map of String to Kind
    List {
        kind: Box<Kind>,
    },
    Optional {
        kind: Box<Kind>,
    },
}

impl Kind {
    pub fn is_optional(&self) -> bool {
        matches!(self, Kind::Optional { .. })
    }

    pub fn is_list(&self) -> bool {
        matches!(self, Kind::List { .. })
    }

    pub fn to_string(&self, line_number: usize, doc_id: &str) -> ftd::p1::Result<String> {
        Ok(match self {
            ftd::p2::Kind::String { .. } => "string",
            ftd::p2::Kind::Integer { .. } => "integer",
            ftd::p2::Kind::Decimal { .. } => "decimal",
            ftd::p2::Kind::Boolean { .. } => "boolean",
            _ => return ftd::e2(format!("Kind supported for default value are string, integer, decimal and boolean with default value, found: kind `{:?}`", &self), doc_id, line_number),
        }.to_string())
    }

    pub fn to_value(&self, line_number: usize, doc_id: &str) -> ftd::p1::Result<ftd::Value> {
        Ok(match self {
            ftd::p2::Kind::String { default: Some(d), .. } => ftd::Value::String {text: d.to_string(), source: ftd::TextSource::Default} ,
            ftd::p2::Kind::Integer { default: Some(d) } => ftd::Value::Integer { value: match d.parse::<i64>() {
                Ok(v) => v,
                Err(_) => return ftd::e2(format!("{} is not an integer", d), doc_id, line_number),
            },},
            ftd::p2::Kind::Decimal { default: Some(d) } => ftd::Value::Decimal { value: d.parse::<f64>().map_err(|e| crate::p1::Error::ParseError {
                message: e.to_string(),
                doc_id: doc_id.to_string(),
                line_number,
            })?, } ,
            ftd::p2::Kind::Boolean { default: Some(d) } => ftd::Value::Boolean { value: d.parse::<bool>().map_err(|e|crate::p1::Error::ParseError {
                message: e.to_string(),
                doc_id: doc_id.to_string(),
                line_number,
            })?, } ,
            _ => return ftd::e2(
                format!("Kind supported for default value are string, integer, decimal and boolean with default value, found: kind `{:?}`", &self),
                doc_id,  line_number),
        })
    }
}

impl Kind {
    pub fn is_same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::String { .. }, Self::String { .. }) => matches!(other, Self::String { .. }),
            (Self::Optional { kind }, _) => kind.is_same_as(other),
            (_, Self::Optional { kind: other }) => self.is_same_as(other),
            _ => self.without_default() == other.without_default(),
        }
    }

    pub fn without_default(&self) -> Self {
        match self {
            Kind::Integer { .. } => Kind::Integer { default: None },
            Kind::Boolean { .. } => Kind::Boolean { default: None },
            Kind::Decimal { .. } => Kind::Decimal { default: None },
            Kind::String { caption, body, .. } => Kind::String {
                caption: *caption,
                body: *body,
                default: None,
            },
            _ => self.clone(),
        }
    }

    pub fn integer() -> Self {
        Kind::Integer { default: None }
    }

    pub fn decimal() -> Self {
        Kind::Decimal { default: None }
    }

    pub fn boolean() -> Self {
        Kind::Boolean { default: None }
    }

    pub fn string() -> Self {
        Kind::String {
            caption: false,
            body: false,
            default: None,
        }
    }
    pub fn get_default_value_str(&self) -> Option<String> {
        match self {
            Kind::Integer { default } => default,
            Kind::Boolean { default } => default,
            Kind::Decimal { default } => default,
            Kind::String { default, .. } => default,
            _ => &None,
        }
        .clone()
    }

    pub fn set_default(self, default: Option<String>) -> Self {
        match self {
            Kind::String { caption, body, .. } => Kind::String {
                caption,
                body,
                default,
            },
            Kind::Integer { .. } => Kind::Integer { default },
            Kind::Decimal { .. } => Kind::Decimal { default },
            Kind::Boolean { .. } => Kind::Boolean { default },
            Kind::Optional { kind } => Kind::Optional {
                kind: Box::from(kind.set_default(default)),
            },
            _ => self,
        }
    }

    pub fn caption() -> Self {
        Kind::String {
            caption: true,
            body: false,
            default: None,
        }
    }

    pub fn body() -> Self {
        Kind::String {
            caption: false,
            body: true,
            default: None,
        }
    }

    pub fn caption_or_body() -> Self {
        Kind::String {
            caption: true,
            body: true,
            default: None,
        }
    }

    pub fn optional(k: Self) -> Self {
        Kind::Optional { kind: Box::new(k) }
    }

    pub fn list(k: Self) -> Self {
        Kind::List { kind: Box::new(k) }
    }

    pub fn map(k: Self) -> Self {
        Kind::Map { kind: Box::new(k) }
    }

    pub fn into_optional(self) -> Self {
        Kind::Optional {
            kind: Box::new(self),
        }
    }

    pub fn is_boolean(&self) -> bool {
        matches!(self, Kind::Boolean { .. })
    }

    pub fn inner(&self) -> &Self {
        match self {
            Kind::Optional { kind } => kind,
            _ => self,
        }
    }

    pub fn list_kind(&self) -> &Self {
        match self {
            Kind::List { kind } => kind,
            _ => self,
        }
    }

    pub fn string_any(&self) -> Self {
        match self {
            Self::String { .. } => Self::String {
                caption: true,
                body: true,
                default: None,
            },
            _ => self.to_owned(),
        }
    }

    pub fn read_section(
        &self,
        line_number: usize,
        p1: &crate::p1::Header,
        p1_caption: &Option<String>,
        p1_body: &Option<(usize, String)>,
        name: &str,
        doc: &crate::p2::TDoc,
    ) -> crate::p1::Result<crate::PropertyValue> {
        let (v, source) = match p1.str_optional(doc.name, line_number, name)? {
            Some(v) => (v.to_string(), crate::TextSource::Header),
            None => {
                let optional = match self {
                    Kind::Optional { kind } => match kind.as_ref() {
                        crate::p2::Kind::String { .. }
                        | crate::p2::Kind::Integer { .. }
                        | crate::p2::Kind::Decimal { .. }
                        | crate::p2::Kind::Boolean { .. } => true,
                        _ => {
                            return Ok(crate::PropertyValue::Value {
                                value: crate::Value::None {
                                    kind: *kind.clone(),
                                },
                            })
                        }
                    },
                    crate::p2::Kind::String { .. }
                    | crate::p2::Kind::Integer { .. }
                    | crate::p2::Kind::Decimal { .. }
                    | crate::p2::Kind::Boolean { .. } => false,
                    t => return ftd::e2(format!("`{}` is {:?}", name, t), doc.name, line_number),
                };

                let (caption, body) = if let Kind::String { caption, body, .. } = self.inner() {
                    (*caption, *body)
                } else {
                    (false, false)
                };

                if caption && p1_caption.is_some() {
                    (
                        p1_caption.as_ref().expect("asd").to_string(),
                        crate::TextSource::Caption,
                    )
                } else if body && p1_body.is_some() {
                    (
                        p1_body.as_ref().expect("asd").1.to_string(),
                        crate::TextSource::Body,
                    )
                } else if optional {
                    return Ok(crate::PropertyValue::Value {
                        value: crate::Value::None {
                            kind: self.inner().to_owned(),
                        },
                    });
                } else if let Some(default) = self.get_default_value_str() {
                    (default, crate::TextSource::Default)
                } else {
                    return ftd::e2(format!("`{}` is required", name), doc.name, line_number);
                }
            }
        };

        if v.starts_with('$') {
            return ftd::PropertyValue::resolve_value(
                line_number,
                &v,
                Some(self.to_owned()),
                doc,
                &Default::default(),
                None,
            );
        }

        match self.inner() {
            Kind::Integer { .. } => Ok(crate::PropertyValue::Value {
                value: crate::Value::Integer {
                    value: p1.i64(doc.name, line_number, name).unwrap_or(
                        v.parse::<i64>().map_err(|e| crate::p1::Error::ParseError {
                            message: e.to_string(),
                            doc_id: doc.name.to_string(),
                            line_number,
                        })?,
                    ),
                },
            }),
            Kind::Decimal { .. } => Ok(crate::PropertyValue::Value {
                value: crate::Value::Decimal {
                    value: p1.f64(doc.name, line_number, name).unwrap_or(
                        v.parse::<f64>().map_err(|e| crate::p1::Error::ParseError {
                            message: e.to_string(),
                            doc_id: doc.name.to_string(),
                            line_number,
                        })?,
                    ),
                },
            }),
            Kind::Boolean { .. } => Ok(crate::PropertyValue::Value {
                value: crate::Value::Boolean {
                    value: p1.bool(doc.name, line_number, name).unwrap_or(
                        v.parse::<bool>()
                            .map_err(|e| crate::p1::Error::ParseError {
                                message: e.to_string(),
                                doc_id: doc.name.to_string(),
                                line_number,
                            })?,
                    ),
                },
            }),
            Kind::String { .. } => Ok(crate::PropertyValue::Value {
                value: crate::Value::String { text: v, source },
            }),
            v => ftd::e2(
                format!("unknown kind found: {:?}", v),
                doc.name,
                line_number,
            ),
        }
    }

    pub fn from(
        line_number: usize,
        s: &str,
        doc: &crate::p2::TDoc,
        object_kind: Option<(&str, Self)>,
    ) -> crate::p1::Result<Self> {
        let (optional, k) = if s.starts_with("optional ") {
            (true, ftd::get_name("optional", s, doc.name)?)
        } else {
            (false, s)
        };

        if k.starts_with("list ") {
            return Ok(Kind::List {
                kind: Box::new(Self::from(
                    line_number,
                    ftd::get_name("list", k, doc.name)?,
                    doc,
                    object_kind,
                )?),
            });
        }

        if let Some((obj_name, obj_kind)) = object_kind {
            if k == obj_name {
                return Ok(obj_kind);
            }
        }

        let (key, default) = {
            if k.contains("with default") {
                let mut parts = k.splitn(2, " with default");
                let k = parts.next().unwrap().trim();
                let d = parts.next().unwrap().trim();
                (k, Some(d.to_string()))
            } else {
                (k, None)
            }
        };

        let k = match key {
            "string" => Kind::string(),
            "caption" => Kind::caption(),
            "body" => Kind::body(),
            "body or caption" | "caption or body" => Kind::caption_or_body(),
            "integer" => Kind::integer(),
            "decimal" => Kind::decimal(),
            "boolean" => Kind::boolean(),
            "element" => Kind::Element,
            "elements" => Kind::Elements,
            "message" => Kind::Message,
            "string-message" => Kind::StringMessage,
            "int-message" => Kind::IntMessage,
            _ => match doc.get_thing(line_number, k)? {
                crate::p2::Thing::Record(r) => Kind::Record { name: r.name },
                crate::p2::Thing::OrType(e) => Kind::OrType { name: e.name },
                t => unimplemented!(
                    "{} is {:?}, line number: {}, doc: {}",
                    k,
                    t,
                    line_number,
                    doc.name.to_string()
                ),
            },
        }
        .set_default(default);

        Ok(if optional { Self::optional(k) } else { k })
    }

    pub fn for_variable(
        line_number: usize,
        s: &str,
        default: Option<String>,
        doc: &crate::p2::TDoc,
        object_kind: Option<(&str, Self)>,
        is_variable: bool,
    ) -> crate::p1::Result<Self> {
        let var_data =
            ftd::variable::VariableData::get_name_kind(s, doc.name, line_number, is_variable)?;

        let kind = var_data.kind.clone().unwrap_or_else(|| "".to_string());

        let k = match object_kind {
            Some(object_kind) if kind.eq(object_kind.0) => object_kind.1,
            _ => match kind.as_str() {
                "string" => Kind::string(),
                "caption" => Kind::caption(),
                "body" => Kind::body(),
                "body or caption" | "caption or body" => Kind::caption_or_body(),
                "integer" => Kind::integer(),
                "decimal" => Kind::decimal(),
                "boolean" => Kind::boolean(),
                "element" => Kind::Element,
                "elements" => Kind::Elements,
                "message" => Kind::Message,
                "string-message" => Kind::StringMessage,
                "int-message" => Kind::IntMessage,
                k => match doc.get_thing(line_number, k) {
                    Ok(crate::p2::Thing::Record(r)) => Kind::Record { name: r.name },
                    Ok(crate::p2::Thing::OrType(e)) => Kind::OrType { name: e.name },
                    t => match default {
                        None => unimplemented!(
                            "{} is {:?}, line number: {}, doc: {}",
                            var_data.name,
                            t,
                            line_number,
                            doc.name.to_string()
                        ),
                        Some(ref d) => ftd::variable::guess_type(d, false)?.kind(),
                    },
                },
            },
        }
        .set_default(default);

        if var_data.is_list() {
            return Ok(Kind::List { kind: Box::new(k) });
        }

        Ok(if var_data.is_optional() {
            Self::optional(k)
        } else {
            k
        })
    }
}