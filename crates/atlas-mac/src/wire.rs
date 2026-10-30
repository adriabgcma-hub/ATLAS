use crate::{Error, Result};
use serde::{Deserialize, Deserializer, Serialize};

pub const MAX_BODY_BYTES: usize = 128 * 1024;
pub const MAX_ECHO_BYTES: usize = 256;
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_PATH_BYTES: usize = 1024;
pub const MAX_WRITE_BYTES: usize = 16 * 1024;
pub const MAX_HASH_HEX: usize = 64;
pub const LEASE_MILLIS: u64 = 15_000;
pub const MAX_WIRE_INTEGER: u64 = i64::MAX as u64;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OpaqueId(String);

impl OpaqueId {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 80
            || !value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Err(Error::InvalidId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for OpaqueId {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<OpaqueId> for String {
    fn from(value: OpaqueId) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct ProtocolVersion;

impl TryFrom<u64> for ProtocolVersion {
    type Error = Error;
    fn try_from(value: u64) -> Result<Self> {
        if value == 3 {
            Ok(Self)
        } else {
            Err(Error::InvalidVersion)
        }
    }
}

impl From<ProtocolVersion> for u64 {
    fn from(_: ProtocolVersion) -> Self {
        3
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct Generation(u64);

impl Generation {
    pub fn new(value: u64) -> Result<Self> {
        if value > MAX_WIRE_INTEGER {
            Err(Error::InvalidGeneration)
        } else {
            Ok(Self(value))
        }
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn checked_next(self) -> Result<Self> {
        Self::new(self.0.checked_add(1).ok_or(Error::InvalidGeneration)?)
    }
}

impl TryFrom<u64> for Generation {
    type Error = Error;
    fn try_from(value: u64) -> Result<Self> {
        Self::new(value)
    }
}

impl From<Generation> for u64 {
    fn from(value: Generation) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct UnixMillis(u64);

impl UnixMillis {
    pub fn new(value: u64) -> Result<Self> {
        if value == 0 || value > MAX_WIRE_INTEGER {
            Err(Error::InvalidExpiration)
        } else {
            Ok(Self(value))
        }
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn checked_add(self, millis: u64) -> Result<Self> {
        Self::new(self.0.checked_add(millis).ok_or(Error::InvalidExpiration)?)
    }
}

impl TryFrom<u64> for UnixMillis {
    type Error = Error;
    fn try_from(value: u64) -> Result<Self> {
        Self::new(value)
    }
}

impl From<UnixMillis> for u64 {
    fn from(value: UnixMillis) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EchoText(String);

impl EchoText {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() > MAX_ECHO_BYTES {
            Err(Error::InvalidText)
        } else {
            Ok(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for EchoText {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<EchoText> for String {
    fn from(value: EchoText) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EchoPayload {
    pub text: EchoText,
}

impl EchoPayload {
    pub fn new(text: impl Into<String>) -> Result<Self> {
        Ok(Self {
            text: EchoText::new(text)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ToolPath(String);

impl ToolPath {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_PATH_BYTES
            || !value.starts_with('/')
            || value.contains('\0')
            || value.chars().any(char::is_control)
        {
            return Err(Error::InvalidPath);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ToolPath {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<ToolPath> for String {
    fn from(value: ToolPath) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathPayload {
    pub path: ToolPath,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FileText(String);

impl FileText {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() > MAX_WRITE_BYTES || value.contains('\0') {
            return Err(Error::InvalidText);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for FileText {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<FileText> for String {
    fn from(value: FileText) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentHash(String);

impl ContentHash {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
        if value.len() != MAX_HASH_HEX || !value.bytes().all(hex) {
            return Err(Error::InvalidHash);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ContentHash {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<ContentHash> for String {
    fn from(value: ContentHash) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WritePayload {
    pub path: ToolPath,
    pub content: FileText,
    #[serde(deserialize_with = "required_nullable")]
    pub expected_hash: Option<ContentHash>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenamePayload {
    pub from: ToolPath,
    pub to: ToolPath,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolArguments {
    Echo(EchoPayload),
    Path(PathPayload),
    Write(WritePayload),
    Rename(RenamePayload),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TextOutput(String);

impl TextOutput {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() > MAX_OUTPUT_BYTES {
            return Err(Error::InvalidText);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TextOutput {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<TextOutput> for String {
    fn from(value: TextOutput) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextPayload {
    pub text: TextOutput,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileInfoOutput {
    pub exists: bool,
    #[serde(deserialize_with = "required_nullable")]
    pub size: Option<u64>,
    #[serde(deserialize_with = "required_nullable")]
    pub sha256: Option<ContentHash>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OutputPayload {
    Text(TextPayload),
    Info(FileInfoOutput),
}

impl OutputPayload {
    pub fn text(value: impl Into<String>) -> Result<Self> {
        Ok(Self::Text(TextPayload {
            text: TextOutput::new(value)?,
        }))
    }

    pub fn text_value(&self) -> Option<&str> {
        match self {
            Self::Text(payload) => Some(payload.text.as_str()),
            Self::Info(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ToolId {
    #[serde(rename = "hefesto.echo")]
    Echo,
    #[serde(rename = "hefesto.list_dir")]
    ListDir,
    #[serde(rename = "hefesto.read_file")]
    ReadFile,
    #[serde(rename = "hefesto.write_file")]
    WriteFile,
    #[serde(rename = "hefesto.rename")]
    Rename,
    #[serde(rename = "hefesto.file_info")]
    FileInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlAction {
    Resume,
    Abort,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InvocationState {
    Pending,
    Dispatched,
    Succeeded,
    Cancelled,
    Expired,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterRequest {
    pub protocol_version: ProtocolVersion,
    pub instance_id: OpaqueId,
}

pub type HeartbeatRequest = RegisterRequest;

impl RegisterRequest {
    pub fn new(instance_id: OpaqueId) -> Self {
        Self {
            protocol_version: ProtocolVersion,
            instance_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlRequest {
    pub protocol_version: ProtocolVersion,
    pub action: ControlAction,
    pub expected_generation: Generation,
}

impl ControlRequest {
    pub fn new(action: ControlAction, expected_generation: Generation) -> Self {
        Self {
            protocol_version: ProtocolVersion,
            action,
            expected_generation,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitRequest {
    pub protocol_version: ProtocolVersion,
    pub submission_id: OpaqueId,
    pub request_id: OpaqueId,
    pub session_id: OpaqueId,
    pub task_id: OpaqueId,
    pub tool_id: ToolId,
    pub arguments: ToolArguments,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NextRequest {
    pub protocol_version: ProtocolVersion,
    pub instance_id: OpaqueId,
    pub generation: Generation,
}

impl NextRequest {
    pub fn new(instance_id: OpaqueId, generation: Generation) -> Self {
        Self {
            protocol_version: ProtocolVersion,
            instance_id,
            generation,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResultRequest {
    pub protocol_version: ProtocolVersion,
    pub instance_id: OpaqueId,
    pub generation: Generation,
    pub invocation_id: OpaqueId,
    pub output: OutputPayload,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Invocation {
    pub protocol_version: ProtocolVersion,
    pub invocation_id: OpaqueId,
    pub request_id: OpaqueId,
    pub session_id: OpaqueId,
    pub task_id: OpaqueId,
    pub device_id: OpaqueId,
    pub instance_id: OpaqueId,
    pub generation: Generation,
    pub tool_id: ToolId,
    pub arguments: ToolArguments,
    pub expires_at_unix_ms: UnixMillis,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, try_from = "StatusWire")]
pub struct Status {
    pub protocol_version: ProtocolVersion,
    pub device_id: OpaqueId,
    pub instance_id: Option<OpaqueId>,
    pub generation: Generation,
    pub suspended: bool,
    pub online: bool,
    pub active_invocation_id: Option<OpaqueId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StatusWire {
    protocol_version: ProtocolVersion,
    device_id: OpaqueId,
    #[serde(deserialize_with = "required_nullable")]
    instance_id: Option<OpaqueId>,
    generation: Generation,
    suspended: bool,
    online: bool,
    #[serde(deserialize_with = "required_nullable")]
    active_invocation_id: Option<OpaqueId>,
}

fn required_nullable<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

impl TryFrom<StatusWire> for Status {
    type Error = Error;
    fn try_from(value: StatusWire) -> Result<Self> {
        let status = Self {
            protocol_version: value.protocol_version,
            device_id: value.device_id,
            instance_id: value.instance_id,
            generation: value.generation,
            suspended: value.suspended,
            online: value.online,
            active_invocation_id: value.active_invocation_id,
        };
        status.validate()?;
        Ok(status)
    }
}

impl Status {
    pub fn validate(&self) -> Result<()> {
        if (self.instance_id.is_none() && (self.online || !self.suspended))
            || (self.generation.get() == 0 && self.instance_id.is_some())
            || (!self.online && !self.suspended)
            || (self.active_invocation_id.is_some() && (!self.online || self.suspended))
        {
            return Err(Error::InvalidState);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvocationResponse {
    pub protocol_version: ProtocolVersion,
    pub invocation_id: OpaqueId,
    pub state: InvocationState,
}

pub type SubmitResponse = InvocationResponse;
pub type ResultResponse = InvocationResponse;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NextResponse {
    pub protocol_version: ProtocolVersion,
    #[serde(deserialize_with = "required_nullable")]
    pub invocation: Option<Invocation>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RemoteErrorCode(String);

impl TryFrom<String> for RemoteErrorCode {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        if value.is_empty()
            || value.len() > 80
            || !value
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        {
            Err(Error::InvalidErrorCode)
        } else {
            Ok(Self(value))
        }
    }
}

impl From<RemoteErrorCode> for String {
    fn from(value: RemoteErrorCode) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorResponse {
    pub protocol_version: ProtocolVersion,
    pub error: RemoteErrorCode,
}

pub fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.len() > MAX_BODY_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| Error::InvalidJson)
}
