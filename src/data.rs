#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Address {
    #[prost(int32, tag = "1")]
    pub ton: i32,
    #[prost(int32, tag = "2")]
    pub npi: i32,
    #[prost(string, tag = "3")]
    pub value: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Message {
    #[prost(string, tag = "1")]
    pub id: ::prost::alloc::string::String,
    #[prost(enumeration = "Charset", tag = "2")]
    pub charset: i32,
    #[prost(bytes = "vec", tag = "3")]
    pub text: ::prost::alloc::vec::Vec<u8>,
    #[prost(message, optional, tag = "4")]
    pub src: ::core::option::Option<Address>,
    #[prost(message, optional, tag = "5")]
    pub dest: ::core::option::Option<Address>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum Charset {
    Iso88591 = 0,
    Iso885915 = 1,
    Gsm7 = 2,
    Gsm8 = 3,
    Utf8 = 4,
    Ucs2 = 5,
}
impl Charset {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            Charset::Iso88591 => "ISO_8859_1",
            Charset::Iso885915 => "ISO_8859_15",
            Charset::Gsm7 => "GSM7",
            Charset::Gsm8 => "GSM8",
            Charset::Utf8 => "UTF8",
            Charset::Ucs2 => "UCS2",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "ISO_8859_1" => Some(Self::Iso88591),
            "ISO_8859_15" => Some(Self::Iso885915),
            "GSM7" => Some(Self::Gsm7),
            "GSM8" => Some(Self::Gsm8),
            "UTF8" => Some(Self::Utf8),
            "UCS2" => Some(Self::Ucs2),
            _ => None,
        }
    }
}
