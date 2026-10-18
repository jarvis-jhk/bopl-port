//! Per-player inputs on the wire.
//!
//! GGRS wants `Copy + PartialEq + Default + Serialize + DeserializeOwned`
//! inputs. The game supplies a plain `bytemuck::Pod` struct instead (bopl-sim's
//! per-player input); [`Wire`] adapts it: the bytes of the struct are the wire
//! format (a fixed number of bytes, no length prefix), `Default` is all zeroes
//! and equality is byte equality. All-zero therefore means "no input", which
//! is also what a dropped player gets.

use bytemuck::Pod;
use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::marker::PhantomData;

/// The inputs of one player for one frame, as the network sees them.
///
/// Implemented for every `Pod` type. Keep it small (GGRS sends the last few
/// frames of every local player in each packet) and state-based (buttons held,
/// not pressed: the sim derives presses by comparing with the previous
/// frame): the predictor repeats the last input. Quantise analog values
/// (Bopl's original sends the stick as one angle byte) so that stick wobble
/// does not cause mispredictions.
pub trait NetInput: Pod + Send + 'static {}
impl<T: Pod + Send + 'static> NetInput for T {}

/// GGRS input wrapper around a `Pod` input.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Wire<I: NetInput>(pub I);

impl<I: NetInput> Default for Wire<I> {
    fn default() -> Self {
        Wire(I::zeroed())
    }
}

impl<I: NetInput> PartialEq for Wire<I> {
    fn eq(&self, other: &Self) -> bool {
        bytemuck::bytes_of(&self.0) == bytemuck::bytes_of(&other.0)
    }
}

impl<I: NetInput> fmt::Debug for Wire<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Wire(")?;
        for b in bytemuck::bytes_of(&self.0) {
            write!(f, "{b:02x}")?;
        }
        write!(f, ")")
    }
}

impl<I: NetInput> Serialize for Wire<I> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let bytes = bytemuck::bytes_of(&self.0);
        let mut t = s.serialize_tuple(bytes.len())?;
        for b in bytes {
            t.serialize_element(b)?;
        }
        t.end()
    }
}

impl<'de, I: NetInput> Deserialize<'de> for Wire<I> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<I>(PhantomData<I>);
        impl<'de, I: NetInput> Visitor<'de> for V<I> {
            type Value = Wire<I>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{} input bytes", std::mem::size_of::<I>())
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Wire<I>, A::Error> {
                let mut v = I::zeroed();
                let bytes = bytemuck::bytes_of_mut(&mut v);
                for (i, b) in bytes.iter_mut().enumerate() {
                    *b = seq
                        .next_element()?
                        .ok_or_else(|| de::Error::invalid_length(i, &self))?;
                }
                Ok(Wire(v))
            }
        }
        d.deserialize_tuple(std::mem::size_of::<I>(), V(PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
    struct In4 {
        angle: u8,
        buttons: u8,
        keys: u8,
        reserved: u8,
    }

    #[test]
    fn fixed_size_roundtrip() {
        let i = Wire(In4 {
            angle: 200,
            buttons: 0b101,
            keys: 3,
            reserved: 0,
        });
        let b = bincode::serialize(&i).unwrap();
        assert_eq!(b.len(), 4, "no length prefix");
        let back: Wire<In4> = bincode::deserialize(&b).unwrap();
        assert_eq!(back, i);
        assert_eq!(
            bincode::serialized_size(&Wire::<In4>::default()).unwrap(),
            4
        );
        assert!(bincode::deserialize::<Wire<In4>>(&b[..3]).is_err());
        assert_eq!(format!("{i:?}"), "Wire(c8050300)");
    }
}
