use std::{str::from_utf8, collections::BTreeMap};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Error {
    EmptyInteger,
    NotEnoughBytes,
    NotAnInteger,
    UnclosedInteger,
    UnclosedList,
    UnclosedMap,
    NegativeZero,
    LeadingZero,
    MissingColon,
    ExpectedMap,
    ExpectedString,
    ExpectedInteger,
    ExpectedList,
}

/// Contains the value and the raw bencode of the type
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Type<'a> {
    String(&'a [u8], &'a [u8]),
    Integer(&'a str, &'a [u8]),
    List(Vec<Type<'a>>, &'a [u8]),
    Map(BTreeMap<Type<'a>, Type<'a>>, &'a [u8])
}

impl<'a> Type<'a> {
    pub fn try_into_dict(&self) -> Result<(&BTreeMap<Type<'a>, Type<'a>>, &'a [u8]), Error> where Self: Sized {
        match self {
            Type::Map(map, raw) => Ok((map, raw)),
            _ => Err(Error::ExpectedMap),
        }
    }

    pub fn try_into_int(&self) -> Result<(&'a str, &'a [u8]), Error> where Self: Sized {
        match self {
            Type::Integer(int, raw) => Ok((int, raw)),
            _ => Err(Error::ExpectedInteger),
        }
    }

    pub fn try_into_list(&self) -> Result<(&Vec<Type<'a>>, &'a [u8]), Error> where Self: Sized {
        match self {
            Type::List(list, raw) => Ok((list, raw)),
            _ => Err(Error::ExpectedList),
        }
    }
    
    pub fn try_into_byte_string(&self) -> Result<(&'a [u8], &'a [u8]), Error> where Self: Sized {
        match self {
            Type::String(string, raw) => Ok((string, raw)),
            _ => Err(Error::ExpectedString),
        }
    }
}

pub trait FromBencodeType {
    type Error;
    fn from_bencode_type(value: &Type) -> Result<Self, Self::Error> where Self: Sized;
}

pub struct Iter<'a> {
    raw: &'a [u8],
    current: usize,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Result<Type<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let len = self.raw.len() - self.current;
        let begin = self.current;

        if len < 2 {
            return None;
        }

        self.current += 1;

        // all bencoding types are represented by valid utf-8 chars 
        // even if their contents aren't
        match self.raw[begin] {
            byte if byte.is_ascii_digit() => {
                while self.current < self.raw.len() && self.raw[self.current].is_ascii_digit() {
                    self.current += 1;
                }

                if self.raw[self.current] != b':' {
                    return Some(Err(Error::MissingColon));
                }

                // consume colon
                self.current += 1;

                let length: usize = from_utf8(&self.raw[begin..(self.current - 1)]).unwrap().parse().unwrap();

                let str_begin = self.current;

                self.current += length;

                let str = &self.raw[str_begin..self.current];

                Some(Ok(Type::String(str, &self.raw[begin..self.current])))
            }
            b'i' => {
                // can't have "ie"
                if self.raw[self.current] == b'e' {
                    return Some(Err(Error::EmptyInteger));
                }

                let mut negative = false;

                // first character may be a negative sign
                if self.raw[self.current] == b'-' {
                    negative = true;
                } else if self.raw[self.current] == b'0' && len > 3 {
                    // leading zeros are not allowed
                    return Some(Err(Error::LeadingZero));
                } else if !self.raw[self.current].is_ascii_digit() {
                    return Some(Err(Error::NotAnInteger));
                }

                if negative {
                    self.current += 1;
                }

                // negative zero is not allowed
                if negative && self.raw[self.current] == b'0' {
                    return Some(Err(Error::NegativeZero));
                }

                // all characters except last one have to be digits
                while self.current < self.raw.len() && self.raw[self.current].is_ascii_digit() {
                    self.current += 1;
                }

                // last character needs to close the integer
                if self.raw[self.current] != b'e' {
                    return Some(Err(Error::UnclosedInteger));
                }

                let str = from_utf8(&self.raw[(begin + 1)..self.current]).unwrap();

                self.current += 1;

                Some(Ok(Type::Integer(str, &self.raw[begin..self.current])))
            }
            b'l' => {
                let mut vec = Vec::new();

                while let Some(object) = self.next() {
                    let object = match object {
                        Ok(object) => object,
                        err => return Some(err),
                    };

                    vec.push(object)
                }

                if self.raw[self.current] != b'e' {
                    return Some(Err(Error::UnclosedList))
                }

                self.current += 1;

                Some(Ok(Type::List(vec, &self.raw[begin..self.current])))
            }
            b'd' => {
                let mut map = BTreeMap::new();

                while let (Some(key), Some(val)) = (self.next(), self.next()) {
                    let key = match key {
                        Ok(key) => key,
                        err => return Some(err),
                    };

                    let val = match val {
                        Ok(val) => val,
                        err => return Some(err),
                    };

                    map.insert(key, val);
                }

                if self.raw[self.current] != b'e' {
                    return Some(Err(Error::UnclosedMap))
                }

                self.current += 1;

                Some(Ok(Type::Map(map, &self.raw[begin..self.current])))
            }
            _ => {
                // revert index
                self.current -= 1;
                None
            }
        }
    }
}

trait BedecodeIter<'a> {
    fn bedecode_iter(self) -> Iter<'a>;
}

impl<'a> BedecodeIter<'a> for &'a [u8] {
    fn bedecode_iter(self) -> Iter<'a> {
        Iter { raw: self, current: 0 }
    }
}

impl<'a, const N: usize> BedecodeIter<'a> for &'a [u8; N] {
    fn bedecode_iter(self) -> Iter<'a> {
        Iter { raw: self, current: 0 }
    }
}

pub trait FromBencode {
    type Error;
    fn from_bencode(bytes: &[u8]) -> Result<Self, Self::Error> where Self: Sized;
}

pub trait Bedecode<'a> {
    fn bedecode(self) -> Result<Type<'a>, Error> where Self: Sized;

    fn try_into_dict(self) -> Result<(BTreeMap<Type<'a>, Type<'a>>, &'a [u8]), Error> where Self: Sized {
        match self.bedecode() {
            Ok(Type::Map(map, raw)) => Ok((map, raw)),
            Ok(_) => Err(Error::ExpectedMap),
            Err(err) => Err(err),
        }
    }

    fn try_into_int(self) -> Result<(&'a str, &'a [u8]), Error> where Self: Sized {
        match self.bedecode() {
            Ok(Type::Integer(int, raw)) => Ok((int, raw)),
            Ok(_) => Err(Error::ExpectedInteger),
            Err(err) => Err(err),
        }
    }

    fn try_into_list(self) -> Result<(Vec<Type<'a>>, &'a [u8]), Error> where Self: Sized {
        match self.bedecode() {
            Ok(Type::List(list, raw)) => Ok((list, raw)),
            Ok(_) => Err(Error::ExpectedList),
            Err(err) => Err(err),
        }
    }
    
    fn try_into_byte_string(self) -> Result<(&'a [u8], &'a [u8]), Error> where Self: Sized {
        match self.bedecode() {
            Ok(Type::String(string, raw)) => Ok((string, raw)),
            Ok(_) => Err(Error::ExpectedString),
            Err(err) => Err(err),
        }
    }
}

impl<'a> Bedecode<'a> for &'a [u8] {
    fn bedecode(self) -> Result<Type<'a>, Error> where Self: Sized {
        self.bedecode_iter().next().ok_or_else(|| Error::NotEnoughBytes)?
    }
}

impl<'a, const N: usize> Bedecode<'a> for &'a [u8; N] {
    fn bedecode(self) -> Result<Type<'a>, Error> where Self: Sized {
        self[..].bedecode()
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use crate::bencode::{Type, Error, Bedecode};

    #[test]
    fn bedecode_string() {
        let str = b"4:spam";
        let empty_str = b"0:";
        let empty = b"";

        assert_eq!(str.bedecode(), Ok(Type::String(b"spam", str)));
        assert_eq!(empty_str.bedecode(), Ok(Type::String(b"", empty_str)));
        assert_eq!(empty.bedecode(), Err(Error::NotEnoughBytes));
    }

    #[test]
    fn bedecode_integer() {
        let positive = b"i10e";
        let negative = b"i-10e";
        let zero = b"i0e";

        assert_eq!(positive.bedecode(), Ok(Type::Integer("10", positive)));
        assert_eq!(negative.bedecode(), Ok(Type::Integer("-10", negative)));
        assert_eq!(zero.bedecode(), Ok(Type::Integer("0", zero)));

        let empty_integer = b"ie";
        let unclosed_integer = b"i10";
        let negative_zero = b"i-0e";
        let leading_zero = b"i03e";
        let negative_leading_zero = b"i-03e";
        
        assert_eq!(empty_integer.bedecode(), Err(Error::EmptyInteger));
        assert_eq!(unclosed_integer.bedecode(), Err(Error::UnclosedInteger));
        assert_eq!(negative_zero.bedecode(), Err(Error::NegativeZero));
        assert_eq!(leading_zero.bedecode(), Err(Error::LeadingZero));
        assert_eq!(negative_leading_zero.bedecode(), Err(Error::NegativeZero));
    }

    #[test]
    fn bedecode_list() {
        let list = b"l4:spam4:eggse";
        let empty = b"le";

        assert_eq!(list.bedecode(), Ok(Type::List(vec![Type::String(b"spam", b"4:spam"), Type::String(b"eggs", b"4:eggs")], list)));
        assert_eq!(empty.bedecode(), Ok(Type::List(vec![], empty)));
    }

    #[test]
    fn bedecode_map() {
        let map_str = b"d3:cow3:moo4:spam4:eggse";
        let map_str2 = b"d4:spaml1:a1:bee";
        let empty = b"de";

        let mut map = BTreeMap::new();
        map.insert(Type::String(b"cow", b"3:cow"), Type::String(b"moo", b"3:moo"));
        map.insert(Type::String(b"spam", b"4:spam"), Type::String(b"eggs", b"4:eggs"));

        let mut map2 = BTreeMap::new();
        map2.insert(Type::String(b"spam", b"4:spam"), Type::List(vec![Type::String(b"a", b"1:a"), Type::String(b"b", b"1:b")], b"l1:a1:be"));

        assert_eq!(map_str.bedecode(), Ok(Type::Map(map, map_str)));
        assert_eq!(map_str2.bedecode(), Ok(Type::Map(map2, map_str2)));
        assert_eq!(empty.bedecode(), Ok(Type::Map(BTreeMap::new(), empty)));
    }
}