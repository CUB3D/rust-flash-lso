use crate::amf3::custom_encoder::ExternalDecoderFn;
use crate::amf3::type_marker::TypeMarker;

use crate::amf3::length::Length;
use crate::nom_utils::AMFResult;
use crate::types::*;
use crate::types::{Element, Value};
use crate::PADDING;
use enumset::EnumSet;
use nom::bytes::complete::tag;
use nom::combinator::map;
use nom::error::{make_error, ErrorKind};
use nom::lib::std::collections::HashMap;
use nom::multi::{many_m_n, separated_list0};
use nom::number::complete::{be_f64, be_i32, be_u32, be_u8};
use nom::take;
use nom::take_str;
use nom::Err;

use std::convert::{TryFrom, TryInto};
use std::ops::DerefMut;
use std::rc::Rc;

const REFERENCE_FLAG: u32 = 0x01;

fn read_int_signed(i: &[u8]) -> AMFResult<'_, i32> {
    let mut vlu_len = 0;
    let mut result: i32 = 0;

    let (mut i, mut v) = be_u8(i)?;
    //TODO: magic numbers from where??
    while v & 0x80 != 0 && vlu_len < 3 {
        result <<= 7;
        result |= (v & 0x7f) as i32;
        vlu_len += 1;

        let (j, m) = be_u8(i)?;
        i = j;
        v = m;
    }

    if vlu_len < 3 {
        result <<= 7;
        result |= v as i32;
    } else {
        result <<= 8;
        result |= v as i32;

        if result & 0x10000000 != 0 {
            result -= 0x20000000;
        }
    }

    Ok((i, result))
}

fn read_int(i: &[u8]) -> AMFResult<'_, u32> {
    let mut n = 0;
    let mut result: u32 = 0;

    let (mut i, mut v) = be_u8(i)?;
    //TODO: magic numbers from where??
    while v & 0x80 != 0 && n < 3 {
        result <<= 7;
        result |= (v & 0x7f) as u32;
        n += 1;

        let (j, v2) = be_u8(i)?;
        i = j;
        v = v2;
    }

    if n < 3 {
        result <<= 7;
        result |= v as u32;
    } else {
        result <<= 8;
        result |= v as u32;

        if result & 0x10000000 != 0 {
            result <<= 1;
            result += 1;
        }
    }

    Ok((i, result))
}

fn read_length(i: &[u8]) -> AMFResult<'_, Length> {
    let (i, val) = read_int(i)?;
    Ok((
        i,
        match val & REFERENCE_FLAG == 0 {
            true => Length::Reference(val as usize >> 1),
            false => Length::Size(val >> 1),
        },
    ))
}

fn parse_element_int(i: &[u8]) -> AMFResult<'_, Rc<Value>> {
    let (i, s) = map(read_int_signed, Value::Integer)(i)?;
    Ok((i, Rc::new(s)))
}

/// Handles decoding AMF3
#[derive(Default)]
pub struct AMF3Decoder {
    /// The table used to cache repeated byte strings
    pub string_reference_table: Vec<Vec<u8>>,
    /// The table used to cache repeated trait definitions
    pub trait_reference_table: Vec<ClassDefinition>,
    /// The table used to cache repeated objects
    pub object_reference_table: Vec<Rc<Value>>,
    /// Encoders used for handling externalized types
    pub external_decoders: HashMap<String, ExternalDecoderFn>,
}

fn parse_element_number(i: &[u8]) -> AMFResult<'_, Rc<Value>> {
    let (i, v) = map(be_f64, Value::Number)(i)?;
    Ok((i, Rc::new(v)))
}

impl AMF3Decoder {
    fn parse_element_string<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Rc<Value>> {
        let (i, s) = map(|i| self.parse_string(i), Value::String)(i)?;
        Ok((i, Rc::new(s)))
    }

    fn parse_string<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, String> {
        let (i, bytes) = self.parse_byte_stream(i)?;
        let bytes_str =
            String::from_utf8(bytes).map_err(|_| Err::Error(make_error(i, ErrorKind::Alpha)))?;
        Ok((i, bytes_str))
    }

    fn parse_class_def<'a>(&mut self, length: u32, i: &'a [u8]) -> AMFResult<'a, ClassDefinition> {
        if length & REFERENCE_FLAG == 0 {
            let len_usize: usize = (length >> 1)
                .try_into()
                .map_err(|_| Err::Error(make_error(i, ErrorKind::Digit)))?;

            let class_def = self
                .trait_reference_table
                .get(len_usize)
                .ok_or_else(|| Err::Error(make_error(i, ErrorKind::Digit)))?
                .clone();

            return Ok((i, class_def));
        }
        let length = length >> 1;

        //TODO: should name be Option<String>
        let (i, name) = self.parse_byte_stream(i)?;
        let name_str = if name.is_empty() {
            "".to_string()
        } else {
            String::from_utf8(name).map_err(|_| Err::Error(make_error(i, ErrorKind::Alpha)))?
        };

        let encoding = (length & 0x03) as u8;

        let attributes_count = length >> 2;

        let attr_count_usize: usize = attributes_count
            .try_into()
            .map_err(|_| Err::Error(make_error(i, ErrorKind::Digit)))?;

        // Read static attributes if they exist
        let (i, static_props) =
            many_m_n(attr_count_usize, attr_count_usize, |i| self.parse_string(i))(i)?;

        let is_external = encoding & 0b1 == 1;
        let is_dynamic = encoding & 0b10 == 0b10;

        let mut attributes = EnumSet::empty();

        if is_external {
            attributes |= Attribute::External;
        }
        if is_dynamic {
            attributes |= Attribute::Dynamic;
        }

        let class_def = ClassDefinition {
            name: name_str,
            attributes,
            static_properties: static_props,
        };

        self.trait_reference_table.push(class_def.clone());
        Ok((i, class_def))
    }

    fn parse_reference_or_val<'a>(
        &mut self,
        i: &'a [u8],
        parser: impl FnOnce(&mut Self, &'a [u8], usize) -> AMFResult<'a, Value>,
    ) -> AMFResult<'a, Rc<Value>> {
        let (i, len) = read_length(i)?;

        match len {
            Length::Reference(index) => {
                let ref_result = Rc::clone(
                    self.object_reference_table
                        .get(index)
                        .ok_or_else(|| Err::Error(make_error(i, ErrorKind::Digit)))?,
                );

                Ok((i, ref_result))
            }
            Length::Size(len) => {
                let len_usize: usize = len
                    .try_into()
                    .map_err(|_| Err::Error(make_error(i, ErrorKind::Digit)))?;

                let initial = Rc::new(Value::Null);
                let index = self.object_reference_table.len();
                self.object_reference_table.push(initial);

                let (i, res) = parser(self, i, len_usize)?;

                //TODO: this should be an error case and also never happen
                let mut initial_inner = Rc::get_mut(
                    self.object_reference_table
                        .get_mut(index)
                        .expect("Index not in reference table"),
                )
                .expect("Reference still held to rc");
                *initial_inner.deref_mut() = res;

                Ok((
                    i,
                    Rc::clone(
                        self.object_reference_table
                            .get(index)
                            .expect("Index not in reference table"),
                    ),
                ))
            }
        }
    }

    fn parse_byte_stream<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Vec<u8>> {
        let (i, len) = read_length(i)?;

        match len {
            Length::Size(len) => {
                if len == 0 {
                    Ok((i, vec![]))
                } else {
                    let (i, bytes) = take!(i, len)?;
                    self.string_reference_table.push(bytes.to_vec());
                    Ok((i, bytes.to_vec()))
                }
            }
            Length::Reference(index) => {
                let ref_result = self
                    .string_reference_table
                    .get(index)
                    .ok_or_else(|| Err::Error(make_error(i, ErrorKind::Digit)))?
                    .clone();

                Ok((i, ref_result))
            }
        }
    }

    fn parse_object_static<'a>(
        &mut self,
        i: &'a [u8],
        class_def: &ClassDefinition,
    ) -> AMFResult<'a, Vec<Element>> {
        let mut elements = Vec::new();
        let mut i = i;

        for name in class_def.static_properties.iter() {
            let (j, e) = self.parse_single_element(i)?;

            elements.push(Element {
                name: name.clone(),
                value: e,
            });

            i = j;
        }

        Ok((i, elements))
    }

    pub(crate) fn parse_element_object<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Rc<Value>> {
        let (i, mut length) = read_int(i)?;

        if length & REFERENCE_FLAG == 0 {
            let len_usize: usize = (length >> 1)
                .try_into()
                .map_err(|_| Err::Error(make_error(i, ErrorKind::Digit)))?;

            let obj = Rc::clone(
                self.object_reference_table
                    .get(len_usize)
                    .ok_or_else(|| Err::Error(make_error(i, ErrorKind::Digit)))?,
            );

            return Ok((i, obj));
        }
        length >>= 1;

        let obj = Rc::new(Value::Object(Vec::new(), None));
        let index = self.object_reference_table.len();
        self.object_reference_table.push(obj);

        // Class def
        let (i, class_def) = self.parse_class_def(length, i)?;

        {
            let mut_obj = Rc::get_mut(
                self.object_reference_table
                    .get_mut(index)
                    .expect("Index invalid"),
            )
            .expect("Unable to get Object");
            if let Value::Object(_, ref mut def) = mut_obj {
                *def = Some(class_def.clone());
            }
        }

        let mut elements = Vec::new();
        let external_elements;

        let mut i = i;
        if class_def.attributes.contains(Attribute::External) {
            return if self.external_decoders.contains_key(&class_def.name) {
                let decoder = Rc::clone(&self.external_decoders[&class_def.name]);
                let (j, v) = decoder(i, self)?;
                external_elements = v;
                i = j;
                //TODO: should it be possible to have both dynamic and external together
                Ok((
                    i,
                    Rc::new(Value::Custom(
                        external_elements,
                        vec![],
                        Some(class_def.clone()),
                    )),
                ))
            } else {
                Err(Err::Error(make_error(i, ErrorKind::Tag)))
            };
        }

        let mut i = i;
        if class_def.attributes.contains(Attribute::Dynamic) {
            let (j, x) = self.parse_object_static(i, &class_def)?;
            elements.extend(x);

            // Read dynamic
            let (mut j, mut attr) = self.parse_byte_stream(j)?;
            while !attr.is_empty() {
                let attr_str = String::from_utf8(attr)
                    .map_err(|_| Err::Error(make_error(i, ErrorKind::Alpha)))?;
                let (k, val) = self.parse_single_element(j)?;
                elements.push(Element {
                    name: attr_str,
                    value: val,
                });

                let (k, attr2) = self.parse_byte_stream(k)?;
                j = k;
                attr = attr2;
            }
            i = j;
        }
        if class_def.attributes.is_empty() {
            let (j, x) = self.parse_object_static(i, &class_def)?;
            elements.extend(x);

            i = j;
        }

        {
            let mut_obj = Rc::get_mut(
                self.object_reference_table
                    .get_mut(index)
                    .expect("Index invalid"),
            )
            .expect("Unable to get Object");
            if let Value::Object(ref mut elements_inner, _) = mut_obj {
                *elements_inner = elements;
            }
        }

        Ok((
            i,
            Rc::clone(
                self.object_reference_table
                    .get(index)
                    .expect("Index invalid"),
            ),
        ))
    }

    fn parse_element_byte_array<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Rc<Value>> {
        self.parse_reference_or_val(i, |_this, i, len| {
            let (i, bytes) = take!(i, len)?;
            Ok((i, Value::ByteArray(bytes.to_vec())))
        })
    }

    fn parse_element_vector_int<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Rc<Value>> {
        self.parse_reference_or_val(i, |_this, i, len| {
            // There must be at least `len * 4` (i32 = 4 bytes) bytes to read this, this prevents OOM errors with v.large vecs
            if i.len() < len * 4 {
                return Err(Err::Error(make_error(i, ErrorKind::TooLarge)));
            }

            let (i, fixed_length) = be_u8(i)?;

            let (i, ints) = many_m_n(len, len, be_i32)(i)?;

            Ok((i, Value::VectorInt(ints, fixed_length == 1)))
        })
    }

    fn parse_element_vector_uint<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Rc<Value>> {
        self.parse_reference_or_val(i, |_this, i, len| {
            // There must be at least `len * 4` (u32 = 4 bytes) bytes to read this, this prevents OOM errors with v.large vecs
            if i.len() < len * 4 {
                return Err(Err::Error(make_error(i, ErrorKind::TooLarge)));
            }
            let (i, fixed_length) = be_u8(i)?;

            let (i, ints) = many_m_n(len, len, be_u32)(i)?;

            Ok((i, Value::VectorUInt(ints, fixed_length == 1)))
        })
    }

    fn parse_element_vector_double<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Rc<Value>> {
        self.parse_reference_or_val(i, |_this, i, len| {
            // There must be at least `len * 8` (f64 = 8 bytes) bytes to read this, this prevents OOM errors with v.large dicts
            if i.len() < len * 8 {
                return Err(Err::Error(make_error(i, ErrorKind::TooLarge)));
            }
            let (i, fixed_length) = be_u8(i)?;

            let (i, numbers) = many_m_n(len, len, be_f64)(i)?;

            Ok((i, Value::VectorDouble(numbers, fixed_length == 1)))
        })
    }

    fn parse_element_object_vector<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Rc<Value>> {
        self.parse_reference_or_val(i, |this, i, len| {
            let (i, fixed_length) = be_u8(i)?;

            let (i, object_type_name) = this.parse_string(i)?;

            let (i, elems) = many_m_n(len, len, |i| this.parse_single_element(i))(i)?;

            Ok((
                i,
                Value::VectorObject(elems, object_type_name, fixed_length == 1),
            ))
        })
    }

    fn parse_element_array<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Rc<Value>> {
        self.parse_reference_or_val(i, |this, i, length_usize| {
            // There must be at least `length_usize` bytes to read this, this prevents OOM errors with v.large dicts
            if i.len() < length_usize {
                return Err(Err::Error(make_error(i, ErrorKind::TooLarge)));
            }

            let (i, mut key) = this.parse_byte_stream(i)?;

            if key.is_empty() {
                let (i, elements) =
                    many_m_n(length_usize, length_usize, |i| this.parse_single_element(i))(i)?;

                return Ok((i, Value::StrictArray(elements)));
            }

            let mut elements = Vec::with_capacity(length_usize);

            let mut i = i;
            while !key.is_empty() {
                let (j, e) = this.parse_single_element(i)?;
                let key_str = String::from_utf8(key)
                    .map_err(|_| Err::Error(make_error(i, ErrorKind::Alpha)))?;

                elements.push(Element {
                    name: key_str,
                    value: e,
                });
                let (j, k) = this.parse_byte_stream(j)?;
                i = j;
                key = k;
            }

            // Must parse `length` elements
            let (i, el) =
                many_m_n(length_usize, length_usize, |i| this.parse_single_element(i))(i)?;

            let elements_len = elements.len() as u32;
            Ok((i, Value::ECMAArray(el, elements, elements_len)))
        })
    }

    fn parse_element_dict<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Rc<Value>> {
        self.parse_reference_or_val(i, |this, i, len| {
            //TODO: implications of this
            let (i, weak_keys) = be_u8(i)?;

            // There must be at least `len * 2` bytes (due to (key,val) pairs) to read this, this prevents OOM errors with v.large dicts
            if i.len() < len * 2 {
                return Err(Err::Error(make_error(i, ErrorKind::TooLarge)));
            }

            let (i, pairs) = many_m_n(len * 2, len * 2, |i| this.parse_single_element(i))(i)?;

            let pairs = pairs
                .chunks_exact(2)
                .map(|chunk| (chunk[0].clone(), chunk[1].clone()))
                .collect::<Vec<_>>();

            Ok((i, Value::Dictionary(pairs, weak_keys == 1)))
        })
    }

    fn parse_element_date<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Rc<Value>> {
        self.parse_reference_or_val(i, |_this, i, _len| {
            let (i, ms) = be_f64(i)?;
            Ok((i, Value::Date(ms, None)))
        })
    }

    fn parse_element_xml<'a>(&mut self, i: &'a [u8], string: bool) -> AMFResult<'a, Rc<Value>> {
        self.parse_reference_or_val(i, |_this, i, len| {
            let (i, data) = take_str!(i, len as u32)?;
            Ok((i, Value::XML(data.into(), string)))
        })
    }

    fn read_type_marker<'a>(&self, i: &'a [u8]) -> AMFResult<'a, TypeMarker> {
        let (i, type_) = be_u8(i)?;
        if let Ok(type_) = TypeMarker::try_from(type_) {
            Ok((i, type_))
        } else {
            Err(Err::Error(make_error(i, ErrorKind::HexDigit)))
        }
    }

    /// Parse a single AMF3 element from the input
    #[inline]
    pub fn parse_single_element<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Rc<Value>> {
        let (i, type_) = self.read_type_marker(i)?;

        match type_ {
            TypeMarker::Undefined => Ok((i, Rc::new(Value::Undefined))),
            TypeMarker::Null => Ok((i, Rc::new(Value::Null))),
            TypeMarker::False => Ok((i, Rc::new(Value::Bool(false)))),
            TypeMarker::True => Ok((i, Rc::new(Value::Bool(true)))),
            TypeMarker::Integer => parse_element_int(i),
            TypeMarker::Number => parse_element_number(i),
            TypeMarker::String => self.parse_element_string(i),
            TypeMarker::XML => self.parse_element_xml(i, false),
            TypeMarker::Date => self.parse_element_date(i),
            TypeMarker::Array => self.parse_element_array(i),
            TypeMarker::Object => self.parse_element_object(i),
            TypeMarker::XmlString => self.parse_element_xml(i, true),
            TypeMarker::ByteArray => self.parse_element_byte_array(i),
            TypeMarker::VectorObject => self.parse_element_object_vector(i),
            TypeMarker::VectorInt => self.parse_element_vector_int(i),
            TypeMarker::VectorUInt => self.parse_element_vector_uint(i),
            TypeMarker::VectorDouble => self.parse_element_vector_double(i),
            TypeMarker::Dictionary => self.parse_element_dict(i),
        }
    }

    fn parse_element<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Element> {
        let (i, name) = self.parse_string(i)?;

        map(
            |i| self.parse_single_element(i),
            move |v| Element {
                name: name.clone(),
                value: v,
            },
        )(i)
    }

    pub(crate) fn parse_body<'a>(&mut self, i: &'a [u8]) -> AMFResult<'a, Vec<Element>> {
        let (i, elements) = separated_list0(tag(PADDING), |i| self.parse_element(i))(i)?;
        let (i, _) = tag(PADDING)(i)?;
        Ok((i, elements))
    }
}