use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeTicket {
    pub lamport: i64,
    pub delimiter: u32,
    pub actor_id: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint {
    pub server_seq: i64,
    pub client_seq: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionVector {
    pub vector: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeId {
    pub client_seq: u32,
    pub server_seq: i64,
    pub lamport: i64,
    pub actor_id: Vec<u8>,
    pub version_vector: Option<VersionVector>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangePack {
    pub document_key: String,
    pub checkpoint: Option<Checkpoint>,
    pub snapshot: Vec<u8>,
    pub changes: Vec<Change>,
    pub is_removed: bool,
    pub version_vector: Option<VersionVector>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: Option<ChangeId>,
    pub message: String,
    pub operations: Vec<Operation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub body: OperationBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationBody {
    Set(OperationSet),
    Add(OperationAdd),
    Move(OperationMove),
    Remove(OperationRemove),
    Increase(OperationIncrease),
    ArraySet(OperationArraySet),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationSet {
    pub parent_created_at: Option<TimeTicket>,
    pub key: String,
    pub value: Option<JsonElementSimple>,
    pub executed_at: Option<TimeTicket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationAdd {
    pub parent_created_at: Option<TimeTicket>,
    pub prev_created_at: Option<TimeTicket>,
    pub value: Option<JsonElementSimple>,
    pub executed_at: Option<TimeTicket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationMove {
    pub parent_created_at: Option<TimeTicket>,
    pub prev_created_at: Option<TimeTicket>,
    pub created_at: Option<TimeTicket>,
    pub executed_at: Option<TimeTicket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRemove {
    pub parent_created_at: Option<TimeTicket>,
    pub created_at: Option<TimeTicket>,
    pub executed_at: Option<TimeTicket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationIncrease {
    pub parent_created_at: Option<TimeTicket>,
    pub value: Option<JsonElementSimple>,
    pub executed_at: Option<TimeTicket>,
    pub actor: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationArraySet {
    pub parent_created_at: Option<TimeTicket>,
    pub created_at: Option<TimeTicket>,
    pub value: Option<JsonElementSimple>,
    pub executed_at: Option<TimeTicket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonElementSimple {
    pub created_at: Option<TimeTicket>,
    pub moved_at: Option<TimeTicket>,
    pub removed_at: Option<TimeTicket>,
    pub value_type: ValueType,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Null,
    Boolean,
    Integer,
    Long,
    Double,
    String,
    Bytes,
    Date,
    JsonObject,
    JsonArray,
    Text,
    IntegerCnt,
    LongCnt,
    IntegerDedupCnt,
    Tree,
}
