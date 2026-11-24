//! Structured messages shared by the domain, storage and startup layers.
//!
//! The backend never builds user-facing prose. It returns a stable key plus
//! typed parameters; the frontend owns the translation. `backend-keys.json`
//! is the contract both sides test against.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

macro_rules! message_keys {
    ($( $variant:ident = $key:literal ),+ $(,)?) => {
        /// Stable identifier of one backend message.
        #[allow(dead_code)] // Some keys are only constructed in debug-only paths.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
        pub enum MessageKey {
            $(#[serde(rename = $key)] $variant),+
        }

        impl MessageKey {
            #[allow(dead_code)] // Used by the catalog test.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $key),+
                }
            }
        }
    };
}

message_keys! {
    NameRequired = "validation.nameRequired",
    UnknownStatus = "validation.unknownStatus",
    UnknownPriority = "validation.unknownPriority",
    ModuleOrderSameProject = "validation.moduleOrderSameProject",
    SubtaskCannotNest = "validation.subtaskCannotNest",
    SubtaskContainerMismatch = "validation.subtaskContainerMismatch",
    SubtaskPriorityAboveParent = "validation.subtaskPriorityAboveParent",
    SubtaskCannotMove = "validation.subtaskCannotMove",
    OnlyTopLevelCanDemote = "validation.onlyTopLevelCanDemote",
    HasChildrenCannotDemote = "validation.hasChildrenCannotDemote",
    CannotDemoteToSelf = "validation.cannotDemoteToSelf",
    SubtaskParentContainerMismatch = "validation.subtaskParentContainerMismatch",
    AlreadyTopLevel = "validation.alreadyTopLevel",
    TodoHasNoModules = "validation.todoHasNoModules",
    SubmoduleNeedsModule = "validation.submoduleNeedsModule",
    ModuleNotInProject = "validation.moduleNotInProject",
    SubmoduleNotInModule = "validation.submoduleNotInModule",
    NotFound = "resource.notFound",
    CascadeDone = "confirm.cascadeDone",
    ParentAutoDone = "notice.parentAutoDone",
    ParentRestoredDoing = "notice.parentRestoredDoing",
    ParentAutoDoing = "notice.parentAutoDoing",
    ParentRestoredTodo = "notice.parentRestoredTodo",
    ParentAutoTodo = "notice.parentAutoTodo",
    DbOperationFailed = "db.operationFailed",
    DbResetPrepareFailed = "db.resetPrepareFailed",
    DbReopenFailed = "db.reopenFailed",
    DbDataDirFailed = "db.dataDirFailed",
    DbCreateDirFailed = "db.createDirFailed",
    DbOpenFailed = "db.openFailed",
    DbForeignKeysFailed = "db.foreignKeysFailed",
    DbBackupCopyFailed = "db.backupCopyFailed",
    DbDeleteFailed = "db.deleteFailed",
    StartupReadVersionFailed = "startup.readVersionFailed",
    StartupNewerDatabase = "startup.newerDatabase",
    StartupDisableForeignKeysFailed = "startup.disableForeignKeysFailed",
    StartupEnableForeignKeysFailed = "startup.enableForeignKeysFailed",
    StartupBeginTransactionFailed = "startup.beginTransactionFailed",
    StartupMigrationFailed = "startup.migrationFailed",
    StartupWriteVersionFailed = "startup.writeVersionFailed",
    StartupCommitFailed = "startup.commitFailed",
    StartupForeignKeyCheckFailed = "startup.foreignKeyCheckFailed",
    StartupForeignKeyCheckReadFailed = "startup.foreignKeyCheckReadFailed",
    StartupForeignKeyViolations = "startup.foreignKeyViolations",
}

/// A parameter value carried by a [`Message`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MessageParam {
    Text(String),
    Number(i64),
    /// A message nested inside another one, e.g. the original failure while
    /// reporting that reopening the database also failed.
    Nested(Box<Message>),
}

impl From<&str> for MessageParam {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for MessageParam {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<i64> for MessageParam {
    fn from(value: i64) -> Self {
        Self::Number(value)
    }
}

impl From<usize> for MessageParam {
    fn from(value: usize) -> Self {
        Self::Number(value as i64)
    }
}

impl From<Message> for MessageParam {
    fn from(value: Message) -> Self {
        Self::Nested(Box::new(value))
    }
}

/// One translatable backend message. Serializes as `{ key, params }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub key: MessageKey,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<&'static str, MessageParam>,
}

impl Message {
    pub fn new(key: MessageKey) -> Self {
        Self {
            key,
            params: BTreeMap::new(),
        }
    }

    pub fn param(mut self, name: &'static str, value: impl Into<MessageParam>) -> Self {
        self.params.insert(name, value.into());
        self
    }
}

impl fmt::Display for Message {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.key.as_str())?;
        if !self.params.is_empty() {
            write!(formatter, " {:?}", self.params)?;
        }
        Ok(())
    }
}

impl std::error::Error for Message {}

pub type Res<T> = Result<T, Message>;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn message_keys_match_backend_catalog() {
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../backend-keys.json")).expect("catalog is JSON");
        let catalog_keys: BTreeSet<&str> = catalog["keys"]
            .as_object()
            .expect("catalog has keys")
            .keys()
            .map(String::as_str)
            .collect();
        let enum_keys: BTreeSet<&str> = MessageKey::ALL.iter().map(|key| key.as_str()).collect();
        assert_eq!(catalog_keys, enum_keys, "Rust keys and the catalog drifted");
    }

    #[test]
    fn message_wire_shape_is_key_and_params() {
        let message = Message::new(MessageKey::NotFound)
            .param("entity", "task")
            .param("id", 7i64);
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["key"], "resource.notFound");
        assert_eq!(json["params"]["entity"], "task");
        assert_eq!(json["params"]["id"], 7);

        let bare = serde_json::to_value(Message::new(MessageKey::SubtaskCannotNest)).unwrap();
        assert_eq!(bare["key"], "validation.subtaskCannotNest");
        assert!(bare.get("params").is_none(), "empty params are omitted");
    }
}
