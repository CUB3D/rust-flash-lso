use cookie_factory::lib::std::fmt::Formatter;
use core::fmt;
use derive_try_from_primitive::TryFromPrimitive;
use enumset::EnumSet;
use enumset::EnumSetType;
use nom::lib::std::iter::FromIterator;
use std::ops::Deref;
use std::rc::Rc;

/// A container for lso files
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, PartialEq, Clone)]
pub struct Lso {
    /// The header of this lso
    pub header: Header,
    /// The elements at the root level of this lso
    pub body: Vec<Element>,
}

impl Lso {
    /// Create a new Lso with a header with the given name and version and an empty body
    #[inline]
    pub fn new_empty(name: impl Into<String>, version: AMFVersion) -> Self {
        Self::new(Vec::new(), name, version)
    }

    /// Crate a new Lso with a header with the given name, version and body
    #[inline]
    pub fn new(body: Vec<Element>, name: impl Into<String>, version: AMFVersion) -> Self {
        Self {
            header: Header::new(name, version),
            body,
        }
    }
}

impl IntoIterator for Lso {
    type Item = Element;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.body.into_iter()
    }
}

/// The version of AMF being used
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(TryFromPrimitive, Eq, PartialEq, Debug, Copy, Clone)]
#[repr(u8)]
pub enum AMFVersion {
    /// AMF0
    AMF0 = 0,
    /// AMF3
    AMF3 = 3,
}

impl fmt::Display for AMFVersion {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            AMFVersion::AMF0 => f.write_str("AMF0"),
            AMFVersion::AMF3 => f.write_str("AMF3"),
        }
    }
}

/// The header of a lso file
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, PartialEq, Clone)]
pub struct Header {
    /// The length of the lso in bytes
    pub length: u32,
    /// The name of the lso file
    pub name: String,
    /// The version of AMF used to encode the data
    pub format_version: AMFVersion,
}

impl Header {
    /// Create a new header with the given name and version, will have a size of 0 by default
    #[inline]
    pub fn new(name: impl Into<String>, version: AMFVersion) -> Self {
        Self {
            length: 0,
            name: name.into(),
            format_version: version,
        }
    }
}

/// Represent a named element
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    /// The name of the element
    pub name: String,
    /// The value of the element
    pub value: Rc<Value>,
}

impl Element {
    /// Create a new Element
    #[inline]
    pub fn new(name: impl Into<String>, value: impl Into<Value>) -> Self {
        Self {
            name: name.into(),
            value: Rc::new(value.into()),
        }
    }

    /// Get the Value of this element
    pub fn value(&self) -> &Value {
        self.value.deref()
    }

    /// Get the name of this element
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
}

//TODO: should amf3 assoc arrays be their own type with a dense and assoc section
/// A single or compound value
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Represent the type number (amf0) and double (amf3)
    Number(f64),
    /// Represents the type boolean (amf0) and both the true/false type (amf3)
    Bool(bool),
    /// Represent both the string (amf0/3) and long string type (amf0)
    String(String),
    /// Represents the object type in both amf0 and amf3, class definition are only available with amf3
    Object(Vec<Element>, Option<ClassDefinition>),
    /// Represent the null type
    Null,
    /// Represent the undefined type
    Undefined,
    /// Represent ECMA-Arrays (amf0) and associative arrays (amf3, even if they contain a dense part)
    /// Final value represents the length of the array in amf0, this can differ from the actual number of elements
    ECMAArray(Vec<Rc<Value>>, Vec<Element>, u32),
    /// Represent a strict array (amf0) or a dense array (amf3)
    StrictArray(Vec<Rc<Value>>),
    /// Represent a timezone in the format (seconds since epoch, timezone or UTC if missing (amf3) )
    Date(f64, Option<u16>),
    /// Represent the unsupported type
    Unsupported,
    /// Represent the XML type, (value, is_string)
    XML(String, bool),
    /// Represent an amf3 element embedded in an AMF0 file
    AMF3(Rc<Value>),
    // AMF3
    /// Represent the integer type (u29) (amf3)
    Integer(i32),
    /// Represent the bytearray type (amf3)
    ByteArray(Vec<u8>),
    /// Represent the int vector type (amf3)
    /// Format is (values, is_fixed_length)
    VectorInt(Vec<i32>, bool),
    /// Represent the unsigned int vector type (amf3)
    /// Format is (values, is_fixed_length)
    VectorUInt(Vec<u32>, bool),
    /// Represent the double vector type (amf3)
    /// Format is (values, is_fixed_length)
    VectorDouble(Vec<f64>, bool),
    /// Represent the object vector type (amf3)
    /// Format is (values, is_fixed_length)
    VectorObject(Vec<Rc<Value>>, String, bool),
    /// Represent the dictionary type (amf3)
    /// Format is ((key, value), has_weak_keys)
    Dictionary(Vec<(Rc<Value>, Rc<Value>)>, bool),
    /// Represent a external object, such as from flex
    /// (custom_elements, regular elements, class def)
    Custom(Vec<Element>, Vec<Element>, Option<ClassDefinition>),
}

impl FromIterator<Value> for Vec<Rc<Value>> {
    fn from_iter<T: IntoIterator<Item = Value>>(iter: T) -> Self {
        iter.into_iter().map(Rc::new).collect()
    }
}

/// A class definition (trait) used in AMF3
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ClassDefinition {
    /// The name of the class definition
    pub name: String,
    /// The attributes on this trait
    pub attributes: EnumSet<Attribute>,
    /// The name of the static properties defined in this definition
    pub static_properties: Vec<String>,
}

impl Default for ClassDefinition {
    fn default() -> Self {
        Self {
            name: "Object".to_string(),
            attributes: EnumSet::empty(),
            static_properties: Vec::new(),
        }
    }
}

impl ClassDefinition {
    /// Creates a new ClassDefinition with the given name, and no attributes or properties
    pub fn default_with_name(name: String) -> Self {
        Self {
            name,
            attributes: EnumSet::empty(),
            static_properties: Vec::new(),
        }
    }
}

/// Encodes the possible attributes that can be given to a trait
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(EnumSetType, Debug)]
pub enum Attribute {
    /// If a trait is dynamic then the object it constructs may have additional properties other than the ones specified in the trait
    Dynamic,
    /// If a trait is external then it requires custom serialization and deserialization support
    External,
}
