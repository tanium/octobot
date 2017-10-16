

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Comment {
    pub body: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Transition {
    pub id: String,
    pub name: String,
    pub to: TransitionTo,
    pub fields: Option<TransitionFields>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct TransitionTo {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct TransitionFields {
    pub resolution: Option<TransitionField<Resolution>>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct TransitionField<T> {
    #[serde(rename = "allowedValues")]
    pub allowed_values: Vec<T>
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Resolution {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct TransitionRequest {
    pub transition: IDOrName,
    pub fields: Option<TransitionFieldsRequest>
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct TransitionFieldsRequest {
    pub resolution: Option<IDOrName>
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct IDOrName {
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Field {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Version {
    pub id: String,
    pub name: String,
}

impl Version {
    pub fn new(name: &str) -> Version {
        Version {
            id: "some-id".into(),
            name: name.into(),
        }
    }
}

impl Transition {
    pub fn new_request(&self) -> TransitionRequest {
        TransitionRequest {
            transition: IDOrName {
                id: Some(self.id.clone()),
                name: None,
            },
            fields: None,
        }
    }
}

impl TransitionRequest {
    pub fn set_resolution(&mut self, res: &Resolution) {
        if self.fields.is_none() {
            self.fields = Some(TransitionFieldsRequest {
                resolution: None,
            });
        }

        if let Some(ref mut fields) = self.fields {
            fields.resolution = Some(IDOrName {
                id: None,
                name: Some(res.name.clone()),
            });
        }
    }
}
