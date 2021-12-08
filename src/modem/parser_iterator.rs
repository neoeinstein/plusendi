
struct BufferProcessor<P, I, O, E> {
    parser: P,
    until_next_parse: usize,
    buffer: bytes::BytesMut,
    _phantom: std::marker::PhantomData<(I, O, E)>
}

impl<P, I, O, E> BufferProcessor<P, I, O, E> {
    fn new(parser: P) -> Self {
        Self {
            parser,
            until_next_parse: 1,
            buffer: bytes::BytesMut::new(),
            _phantom: std::marker::PhantomData,
        }
    }

    fn with_capacity(parser: P, capacity: usize) -> Self {
        Self {
            parser,
            until_next_parse: 1,
            buffer: bytes::BytesMut::with_capacity(capacity),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<P, I, O, E> BufferProcessor<P, I, O, E>
    where
        P: Fn(I) -> IResult<I, O, E>
{
    fn iter(&mut self) -> ParsedIterator<P, I, O, E> {
        ParsedIterator {
            processor: self,
            position: 0,
        }
    }
}

struct ParsedIterator<'a, P, I, O, E> {
    processor: &'a mut BufferProcessor<P, I, O, E>,
    position: usize,
}

impl<'a, I, P, O, E> Drop for ParsedIterator<'a, I, P, O, E> {
    fn drop(&mut self) {
        if self.position == self.processor.buffer.len() {
            self.processor.buffer.clear();
        } else if self.position > 0 {
            let new = self.processor.buffer.split_off(self.position);
            self.processor.buffer = new;
            tracing::trace!(bytes = self.processor.buffer.len(), "retained incomplete parts");
        }
    }
}

impl<'a, P, I, O, E> Iterator for ParsedIterator<'a, P, I, O, E>
    where
        P: Fn(I) -> IResult<I, O, E>,
        I: From<&'a [u8]> + AsRef<[u8]> + 'a,
{
    type Item = Result<O, E>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.processor.until_next_parse > 0 {
            return None;
        }

        let borrowed = &self.processor.buffer[..];

        match (self.processor.parser)(I::from(borrowed)) {
            Ok((remaining, output)) => {
                self.position = self.processor.buffer.len() - remaining.as_ref().len();
                Some(Ok(output))
            }
            Err(nom::Err::Incomplete(nom::Needed::Unknown)) => {
                self.processor.until_next_parse = 1;
                None
            }
            Err(nom::Err::Incomplete(nom::Needed::Size(bytes))) => {
                self.processor.until_next_parse = bytes.get();
                None
            },
            Err(nom::Err::Error(err) | nom::Err::Failure(err)) => {
                Some(Err(err))
            }
        }
    }
}

unsafe impl<P, I, O, E> bytes::BufMut for BufferProcessor<P, I, O, E> {
    #[inline(always)]
    fn remaining_mut(&self) -> usize {
        self.buffer.remaining_mut()
    }

    #[inline(always)]
    unsafe fn advance_mut(&mut self, cnt: usize) {
        self.buffer.advance_mut(cnt);

        // Keep track of bytes that have been added to the buffer so we know
        // when it is reasonable to try parsing again.
        self.until_next_parse.saturating_sub(cnt);
    }

    #[inline(always)]
    fn chunk_mut(&mut self) -> &mut UninitSlice {
        self.buffer.chunk_mut()
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;
    use bytes::BufMut;
    use super::*;

    #[test]
    fn buffer_processor_does_job() {
        let data = "CONNECTED KC1GSL KW1U\rPTT ON\rPTT";
        let mut processor = BufferProcessor::new(line);
        processor.put_slice(data.as_bytes());
        let mut iter = processor.iter();
        assert!(matches!(iter.next(), Some(Ok(l)) if l == "CONNECTED KC1GSL KW1U"));
        assert!(matches!(iter.next(), Some(Ok(l)) if l == "PTT ON"));
        assert!(matches!(iter.next(), Some(None)));
    }
}



Err(err) => {
+                Err(VerboseError {
+                    errors: err.errors.into_iter().map(|e| {
+                        (e.0.to_string(), e.1)
+                    }).collect()
+                })?
}

.map_err(|err| {
            VerboseError {
                errors: err.errors.into_iter().map(|e| {
                    (crate::parser::StrOrByteSlice::Bytes(e.0).to_string(), e.1)
                }).collect()
            }
        }
