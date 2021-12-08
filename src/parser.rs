use std::fmt;
use nom::error::{Error, VerboseError};
use nom::IResult;

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum StrOrByteSlice<'a> {
    Str(&'a str),
    Bytes(&'a [u8]),
}

impl<'a> fmt::Debug for StrOrByteSlice<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write;
        match self {
            Self::Str(s) => fmt::Debug::fmt(s, f),
            Self::Bytes(bs) => {
                f.debug_list()
                    .entries(bs.iter().map(|b| format!("{:#04X}", b)))
                    .finish()
            }
        }
    }
}

impl<'a> fmt::Display for StrOrByteSlice<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write;
        match self {
            Self::Str(s) => fmt::Display::fmt(s, f),
            Self::Bytes(bs) => {
                f.debug_list()
                    .entries(bs.iter().map(|b| format!("{:#04X}", b)))
                    .finish()
            }
        }
    }
}

pub(crate) trait MappableParserInputError {
    type Output;
    fn try_map_into_str(self) -> Self::Output;
}

impl<X> MappableParserInputError for nom::Err<X>
where
    X: MappableParserInputError
{
    type Output = nom::Err<X::Output>;
    fn try_map_into_str(self) -> Self::Output {
        self.map(MappableParserInputError::try_map_into_str)
    }
}

impl<'a, I, O> MappableParserInputError for IResult<I, O, VerboseError<&'a [u8]>> {
    type Output = IResult<I, O, VerboseError<StrOrByteSlice<'a>>>;
    fn try_map_into_str(self) -> Self::Output {
        self.map_err(MappableParserInputError::try_map_into_str)
    }
}

impl<'a> MappableParserInputError for VerboseError<&'a [u8]> {
    type Output = VerboseError<StrOrByteSlice<'a>>;
    fn try_map_into_str(self) -> Self::Output {
        VerboseError {
            errors: self.errors.into_iter().map(|e| {
                let data = std::str::from_utf8(e.0).map(StrOrByteSlice::Str).unwrap_or(StrOrByteSlice::Bytes(e.0));
                (data, e.1)
            }).collect()
        }
    }
}

impl<'a, I, O> MappableParserInputError for IResult<I, O, VerboseError<&'a str>> {
    type Output = IResult<I, O, VerboseError<StrOrByteSlice<'a>>>;
    fn try_map_into_str(self) -> Self::Output {
        self.map_err(MappableParserInputError::try_map_into_str)
    }
}

impl<'a> MappableParserInputError for VerboseError<&'a str> {
    type Output = VerboseError<StrOrByteSlice<'a>>;
    fn try_map_into_str(self) -> Self::Output {
        VerboseError {
            errors: self.errors.into_iter().map(|e| (StrOrByteSlice::Str(e.0), e.1)).collect()
        }
    }
}

impl<'a, I, O> MappableParserInputError for IResult<I, O, Error<&'a [u8]>> {
    type Output = IResult<I, O, Error<StrOrByteSlice<'a>>>;
    fn try_map_into_str(self) -> Self::Output {
        self.map_err(MappableParserInputError::try_map_into_str)
    }
}

impl<'a> MappableParserInputError for Error<&'a [u8]> {
    type Output = Error<StrOrByteSlice<'a>>;
    fn try_map_into_str(self) -> Self::Output {
        Error {
            input: std::str::from_utf8(self.input).map(StrOrByteSlice::Str).unwrap_or(StrOrByteSlice::Bytes(self.input)),
            code: self.code,
        }
    }
}

impl<'a, I, O> MappableParserInputError for IResult<I, O, Error<&'a str>> {
    type Output = IResult<I, O, Error<StrOrByteSlice<'a>>>;
    fn try_map_into_str(self) -> Self::Output {
        self.map_err(MappableParserInputError::try_map_into_str)
    }
}

impl<'a> MappableParserInputError for Error<&'a str> {
    type Output = Error<StrOrByteSlice<'a>>;
    fn try_map_into_str(self) -> Self::Output {
        Error {
            input: StrOrByteSlice::Str(self.input),
            code: self.code,
        }
    }
}
