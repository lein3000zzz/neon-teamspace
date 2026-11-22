//! A database's schema: its properties in order, and the rules for changing them.

use uuid::Uuid;

use super::property::{
    Choice, Property, PropertyKind, PropertyType, check_name, default_status_options,
};
use super::{MAX_PROPERTIES, SchemaError};

/// A database's properties, in their order. Exactly one is the title, and at most one a
/// unique id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schema {
    pub properties: Vec<Property>,
}

impl Schema {
    pub fn get(&self, id: Uuid) -> Option<&Property> {
        self.properties.iter().find(|property| property.id == id)
    }

    /// The property `key` names: by its id, or else by its name, ignoring case.
    pub fn find(&self, key: &str) -> Option<&Property> {
        Uuid::try_parse(key.trim())
            .ok()
            .and_then(|id| self.get(id))
            .or_else(|| {
                let key = key.trim().to_lowercase();
                self.properties
                    .iter()
                    .find(|property| property.name.to_lowercase() == key)
            })
    }

    /// The title property.
    pub fn title(&self) -> Option<&Property> {
        self.of_kind(PropertyKind::Title)
    }

    /// The unique id property, if the database has one.
    pub fn unique_id(&self) -> Option<&Property> {
        self.of_kind(PropertyKind::UniqueId)
    }

    fn of_kind(&self, kind: PropertyKind) -> Option<&Property> {
        self.properties
            .iter()
            .find(|property| property.kind() == kind)
    }

    /// Whether another property than `except` has this name, ignoring case.
    fn name_taken(&self, name: &str, except: Option<Uuid>) -> bool {
        let name = name.to_lowercase();
        self.properties
            .iter()
            .any(|property| Some(property.id) != except && property.name.to_lowercase() == name)
    }

    /// The name and type of a property to add, checked: a name of its own, a type that is
    /// not a second title or unique id, and room for it. Its options, if any, are checked
    /// by the caller with [`check_options`](super::check_options).
    pub fn check_new(
        &self,
        name: &str,
        ty: PropertyType,
    ) -> Result<(String, PropertyType), SchemaError> {
        if self.properties.len() >= MAX_PROPERTIES {
            return Err(SchemaError::TooMany(format!(
                "a database has at most {MAX_PROPERTIES} properties"
            )));
        }
        let name = self.check_name(name, None)?;
        let ty = ty.checked()?;
        self.check_kind(ty.kind(), None)?;
        Ok((name, ty))
    }

    /// A new name for the property `id`, checked.
    pub fn check_rename(&self, id: Uuid, name: &str) -> Result<String, SchemaError> {
        self.check_name(name, Some(id))
    }

    fn check_name(&self, name: &str, except: Option<Uuid>) -> Result<String, SchemaError> {
        let name = check_name(name)?;
        if self.name_taken(&name, except) {
            return Err(SchemaError::NameTaken(format!(
                "the database has a property named {name:?} already"
            )));
        }
        Ok(name)
    }

    /// Whether a property `except` may become of `kind`: the title is made with the
    /// database and stays, and there is at most one unique id.
    fn check_kind(&self, kind: PropertyKind, except: Option<Uuid>) -> Result<(), SchemaError> {
        match kind {
            PropertyKind::Title => Err(SchemaError::Invalid(
                "a database has exactly one title property, made with it".to_owned(),
            )),
            PropertyKind::UniqueId
                if self
                    .unique_id()
                    .is_some_and(|property| Some(property.id) != except) =>
            {
                Err(SchemaError::Invalid(
                    "a database has at most one unique id property".to_owned(),
                ))
            }
            _ => Ok(()),
        }
    }

    /// Whether the property `id` may change to a type of `kind`: the title keeps its type,
    /// and no other becomes the title or a second unique id.
    pub fn check_retype(&self, id: Uuid, kind: PropertyKind) -> Result<(), SchemaError> {
        let property = self.get(id).ok_or(SchemaError::NoProperty)?;
        if property.kind() == PropertyKind::Title {
            return Err(SchemaError::Invalid(
                "the title property keeps its type".to_owned(),
            ));
        }
        if property.kind() == kind {
            return Ok(());
        }
        self.check_kind(kind, Some(id))
    }

    /// Whether the property `id` may be deleted: any but the title.
    pub fn check_delete(&self, id: Uuid) -> Result<&Property, SchemaError> {
        let property = self.get(id).ok_or(SchemaError::NoProperty)?;
        if property.kind() == PropertyKind::Title {
            return Err(SchemaError::Invalid(
                "the title property cannot be deleted".to_owned(),
            ));
        }
        Ok(property)
    }

    /// The options of a select or status property that `options` leaves out: those whose
    /// values the change clears.
    pub fn removed_options(&self, id: Uuid, options: &[Choice]) -> Vec<Uuid> {
        self.get(id)
            .map(|property| {
                property
                    .ty
                    .options()
                    .iter()
                    .filter(|old| options.iter().all(|new| new.id != old.id))
                    .map(|old| old.id)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The properties a database starts with, in order: Name, its title, and Status, with its
/// default options.
pub fn default_properties(ids: &mut dyn FnMut() -> Uuid) -> Vec<Property> {
    vec![
        Property {
            id: ids(),
            name: "Name".to_owned(),
            ty: PropertyType::Title,
        },
        Property {
            id: ids(),
            name: "Status".to_owned(),
            ty: PropertyType::Status {
                options: default_status_options(ids),
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter() -> impl FnMut() -> Uuid {
        let mut next = 0u128;
        move || {
            next += 1;
            Uuid::from_u128(next)
        }
    }

    fn schema() -> Schema {
        Schema {
            properties: default_properties(&mut counter()),
        }
    }

    #[test]
    fn a_database_starts_with_a_title_and_a_status() {
        let schema = schema();
        let names: Vec<&str> = schema.properties.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Name", "Status"]);
        assert_eq!(schema.title().map(|p| p.id), Some(Uuid::from_u128(1)));
        let status = schema.find("status").expect("a status");
        let options: Vec<(&str, &str)> = status
            .ty
            .options()
            .iter()
            .map(|o| (o.name.as_str(), o.group().map_or("", |g| g.as_str())))
            .collect();
        assert_eq!(
            options,
            [
                ("Not started", "to_do"),
                ("In progress", "in_progress"),
                ("Done", "done")
            ]
        );
        assert_eq!(schema.unique_id(), None);
    }

    #[test]
    fn properties_are_found_by_id_or_name() {
        let schema = schema();
        assert_eq!(
            schema
                .find(&Uuid::from_u128(2).to_string())
                .map(|p| p.name.as_str()),
            Some("Status")
        );
        assert_eq!(
            schema.find(" NAME ").map(|p| p.id),
            Some(Uuid::from_u128(1))
        );
        assert_eq!(schema.find("Owner"), None);
        assert_eq!(schema.find(&Uuid::from_u128(77).to_string()), None);
    }

    #[test]
    fn new_properties_need_a_name_of_their_own_and_a_kind_that_fits() {
        let mut schema = schema();
        assert_eq!(
            schema.check_new(" Owner ", PropertyType::Person),
            Ok(("Owner".to_owned(), PropertyType::Person))
        );
        assert!(matches!(
            schema.check_new("status", PropertyType::Text),
            Err(SchemaError::NameTaken(_))
        ));
        assert!(matches!(
            schema.check_new("Second title", PropertyType::Title),
            Err(SchemaError::Invalid(_))
        ));
        assert!(schema.check_new("", PropertyType::Text).is_err());
        assert_eq!(
            schema.check_new(
                "ID",
                PropertyType::UniqueId {
                    prefix: Some("nw".into())
                }
            ),
            Ok((
                "ID".to_owned(),
                PropertyType::UniqueId {
                    prefix: Some("NW".into())
                }
            ))
        );
        schema.properties.push(Property {
            id: Uuid::from_u128(50),
            name: "ID".into(),
            ty: PropertyType::UniqueId { prefix: None },
        });
        assert!(
            schema
                .check_new("Other ID", PropertyType::UniqueId { prefix: None })
                .is_err()
        );
        while schema.properties.len() < MAX_PROPERTIES {
            let n = schema.properties.len();
            schema.properties.push(Property {
                id: Uuid::from_u128(1000 + n as u128),
                name: format!("p{n}"),
                ty: PropertyType::Text,
            });
        }
        assert!(matches!(
            schema.check_new("One more", PropertyType::Text),
            Err(SchemaError::TooMany(_))
        ));
    }

    #[test]
    fn renames_keep_names_apart() {
        let schema = schema();
        let status = Uuid::from_u128(2);
        assert_eq!(schema.check_rename(status, "State"), Ok("State".to_owned()));
        // Its own name in another case is its own.
        assert_eq!(
            schema.check_rename(status, "STATUS"),
            Ok("STATUS".to_owned())
        );
        assert!(matches!(
            schema.check_rename(status, "name"),
            Err(SchemaError::NameTaken(_))
        ));
    }

    #[test]
    fn the_title_keeps_its_type_and_stays() {
        let mut schema = schema();
        let (title, status) = (Uuid::from_u128(1), Uuid::from_u128(2));
        assert!(schema.check_retype(title, PropertyKind::Text).is_err());
        assert!(schema.check_retype(status, PropertyKind::Title).is_err());
        assert!(schema.check_retype(status, PropertyKind::Select).is_ok());
        assert!(schema.check_retype(status, PropertyKind::Status).is_ok());
        assert!(schema.check_retype(status, PropertyKind::UniqueId).is_ok());
        assert_eq!(
            schema.check_retype(Uuid::from_u128(9), PropertyKind::Text),
            Err(SchemaError::NoProperty)
        );
        assert!(schema.check_delete(title).is_err());
        assert!(schema.check_delete(status).is_ok());
        assert_eq!(
            schema.check_delete(Uuid::from_u128(9)).map(|_| ()),
            Err(SchemaError::NoProperty)
        );
        schema.properties.push(Property {
            id: Uuid::from_u128(50),
            name: "ID".into(),
            ty: PropertyType::UniqueId { prefix: None },
        });
        assert!(schema.check_retype(status, PropertyKind::UniqueId).is_err());
        assert!(
            schema
                .check_retype(Uuid::from_u128(50), PropertyKind::UniqueId)
                .is_ok()
        );
    }

    #[test]
    fn options_left_out_are_removed() {
        let schema = schema();
        let status = schema.get(Uuid::from_u128(2)).expect("status").clone();
        let kept: Vec<Choice> = status.ty.options()[1..].to_vec();
        assert_eq!(
            schema.removed_options(status.id, &kept),
            vec![status.ty.options()[0].id]
        );
        assert!(schema.removed_options(Uuid::from_u128(1), &[]).is_empty());
    }
}
