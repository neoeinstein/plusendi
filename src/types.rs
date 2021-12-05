use aliri_braid::braid;
use std::borrow::Cow;
use lazy_regex::{Lazy, lazy_regex};
use regex::Regex;
use thiserror::Error;

static STATION: Lazy<Regex> = lazy_regex!(r#"([0-9]?[A-Za-z]+)([0-9]+)([A-Za-z][A-Za-z0-9]*)"#);

#[derive(Debug, Error)]
#[error("invalid station identity")]
pub struct InvalidStationId;//(#[from] nom::Err<nom::error::Error<String>>);

#[braid(normalizer)]
pub struct StationId;

impl aliri_braid::Normalizer for StationId {
    type Error = InvalidStationId;

    fn normalize(s: &str) -> Result<Cow<str>, Self::Error> {
        // let (rest, cs) = nom::combinator::all_consuming(callsign)(s).map_err(|e| e.to_owned())?;
        // Ok(cs)
        if STATION.is_match(s) {
            if s.as_bytes().iter().any(|&b| b'a' <= b && b <= b'z') {
                Ok(Cow::Owned(s.to_ascii_uppercase()))
            } else {
                Ok(Cow::Borrowed(s))
            }
        } else {
            Err(InvalidStationId)
        }
    }
}

pub fn callsign(s: &[u8]) -> nom::IResult<&[u8], &StationIdRef> {
    let (rest, result) = nom::bytes::complete::take_while_m_n(3,7, |c: u8| c.is_ascii_uppercase() || c.is_ascii_digit())(s)?;
    // let cow = if result.iter().any(|&b| b'a' <= b && b <= b'z') {
    //     Cow::Owned(unsafe { String::from_utf8_unchecked(result.to_ascii_uppercase()) })
    // } else {
    //     Cow::Borrowed(unsafe { std::str::from_utf8_unchecked(result) })
    // };

    Ok((rest, unsafe { StationIdRef::from_str_unchecked(std::str::from_utf8_unchecked(result)) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_callsign() -> color_eyre::Result<()> {
        let x = StationIdRef::from_str("KC1GSL")?;
        assert!(matches!(x, Cow::Borrowed(_)));
        Ok(())
    }

    #[test]
    fn normalized_callsign() -> color_eyre::Result<()> {
        let x = StationIdRef::from_str("kc1gsl")?;
        assert!(matches!(x, Cow::Owned(_)));
        assert_eq!(x.into_owned(), StationId::new("KC1GSL")?);
        Ok(())
    }
}
