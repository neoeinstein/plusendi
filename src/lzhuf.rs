use std::fmt;

const N: u16 = 2048;
const F: u16 = 60;
const THRESHOLD: u16 = 2;
const NIL: u16 = N; // 2048
const N_CHAR: u16 = 256 - THRESHOLD + F; // 314
const T: u16 = N_CHAR * 2 - 1; // 627
const R: u16 = T - 1; // 313
const MAX_FREQ: u16 = 0x8000;

#[derive(Debug)]
struct LzHufState {
    frequency_table: [u16; T as usize + 1],
    parents: [u16; (T + N_CHAR) as usize],
    children: [u16; T as usize],
    text_buffer: [u8; (N + F - 1) as usize],
    r: u16,
    // bit_buffer: u16,
    // bit_buf_len: u8,
}

impl LzHufState {
    fn new() -> Self {
        let mut frequency_table = [0; T as usize + 1];
        let mut parents = [0; (T + N_CHAR) as usize];
        let mut children = [0; T as usize];
        for i in 0..N_CHAR {
            frequency_table[i as usize] = 1;
            children[i as usize] = i + T;
            parents[(i + T) as usize] = i;
        }

        let mut i = 0;
        let mut j = N_CHAR;
        while i + 1 < j {
            frequency_table[j as usize] = frequency_table[i as usize] + frequency_table[i as usize + 1];
            j += 1;
            i += 2;
        }

        let mut i = 0;
        let mut j = N_CHAR;
        while i < T - 1 {
            children[j as usize] = i;
            parents[i as usize] = j;
            parents[i as usize + 1] = j;
            j += 1;
            i += 2;
        }

        frequency_table[T as usize] = 0xffff;
        parents[R as usize] = 0;

        LzHufState {
            frequency_table,
            parents,
            children,
            text_buffer: [0x20; (N + F - 1) as usize],
            r: N - F,
        }
    }

    #[tracing::instrument(skip(self))]
    fn reconstruct(&mut self) {
        let mut j = 0;
        for i in 0..T {
            if self.children[i as usize] >= T {
                self.frequency_table[j as usize] = (self.frequency_table[i as usize] + 1) / 2;
                self.children[j as usize] = self.children[i as usize];
                j += 1;
            }
        }

        for (j, i) in (N_CHAR..T).zip((0..).step_by(2)) {
            let k = i + 1;
            self.frequency_table[j as usize] = self.frequency_table[i as usize] + self.frequency_table[k as usize];
            let mut k = j - 1;
            let f = self.frequency_table[j as usize];
            while f < self.frequency_table[k as usize] {
                k -= 1;
            }
            k += 1;
            let l = (j - k) * 2;
            self.frequency_table.copy_within((k as usize)..((k+l) as usize), k as usize + 1);
            self.frequency_table[k as usize] = f;
            self.children.copy_within((k as usize)..((k+l) as usize), k as usize + 1);
            self.children[k as usize] = i;
        }

        for i in 0..T {
            let k = self.children[i as usize];
            if k >= T {
                self.parents[k as usize] = i;
            } else {
                self.parents[k as usize + 1] = i;
                self.parents[k as usize] = self.parents[k as usize + 1]
            }
        }
    }

    #[tracing::instrument(skip(self))]
    fn update(&mut self, c: u16) {
        if self.frequency_table[R as usize] == MAX_FREQ {
            self.reconstruct()
        }

        let mut c = self.parents[(c + T) as usize];
        loop {
            self.frequency_table[c as usize] += 1;
            let k = self.frequency_table[c as usize];
            let mut l = c + 1;
            if k > self.frequency_table[l as usize] {
                l += 1;
                while k > self.frequency_table[l as usize] {
                    l += 1;
                }
                l -= 1;
                self.frequency_table[c as usize] = self.frequency_table[l as usize];
                self.frequency_table[l as usize] = k;

                let i = self.children[c as usize];
                self.parents[i as usize] = l;
                if i < T {
                    self.parents[i as usize + 1] = l;
                }

                let j = self.children[l as usize];
                self.children[l as usize] = i;

                self.parents[j as usize] = c;
                if j < T {
                    self.parents[j as usize + 1] = c;
                }
                self.children[c as usize] = j;

                c = l;
            }
            c = self.parents[c as usize];
            if c == 0 {
                break;
            }
        }
    }

    #[tracing::instrument(skip(self))]
    fn update_text_buffer(&mut self, c: u8) {
        self.text_buffer[self.r as usize] = c;
        self.r += 1;
        self.r &= N - 1;
    }
}

struct Bitbuffer<'a> {
    bit_buffer: u16,
    bit_pos: u8,
    output: &'a mut Vec<u8>,
}

impl<'a> Bitbuffer<'a> {
    fn new(output: &'a mut Vec<u8>) -> Self {
        Self {
            bit_pos: 0,
            bit_buffer: 0,
            output,
        }
    }

    fn put_code(&mut self, l: u8, c: u16) {
        self.bit_buffer |= c >> self.bit_pos;
        self.bit_pos += l;
        if self.bit_pos >= 8 {
            self.output.push((self.bit_buffer >> 8) as u8);
            self.bit_pos -= 8;
            if self.bit_pos >= 8 {
                self.output.push(self.bit_buffer as u8);
                // self.codesize += 2;
                self.bit_pos -= 8;
                self.bit_buffer = c << (l - self.bit_pos) as usize;
            } else {
                self.bit_buffer = self.bit_buffer << 8;
                // self.codesize += 1;
            }
        }
    }

    fn finish(self) {}
}

impl<'a> Drop for Bitbuffer<'a> {
    fn drop(&mut self) {
        if self.bit_pos > 0 {
            self.output.push((self.bit_buffer >> 8) as u8);
        }
    }
}

struct Biterator<I> {
    bit_buffer: u32,
    bit_pos: u8,
    input: I
}

impl<I> Biterator<I> {
    fn new<X: IntoIterator<IntoIter = I, Item = u8>>(input: X) -> Self {
        Self {
            bit_pos: 0,
            bit_buffer: 0,
            input: input.into_iter(),
        }
    }
}

impl<I> fmt::Debug for Biterator<I> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Biterator")
            .field("bit_buffer", &format_args!("{:#034b}", self.bit_buffer))
            .field("bit_pos", &self.bit_pos)
            .field("input", &"…")
            .finish()
    }
}

impl<I: Iterator<Item = u8>> Biterator<I> {
    #[tracing::instrument(skip(self))]
    fn fill_buffer(&mut self) {
        while self.bit_pos <= 8 {
            if let Some(inter) = self.input.next() {
                let inter = inter as i16;
                let i = if inter < 0 { 0 } else { inter as u32 };
                self.bit_buffer |= i << (8 - self.bit_pos);
                self.bit_pos += 8;
            } else {
                break;
            }
        }
    }

    #[tracing::instrument(skip(self))]
    fn get_bit(&mut self) -> Option<u8> {
        self.fill_buffer();

        if self.bit_pos == 0 {
            return None;
        }

        let i = self.bit_buffer;
        self.bit_buffer = self.bit_buffer << 1;
        self.bit_pos -= 1;

        Some(((i & 0x8000) >> 15) as u8)
    }

    #[tracing::instrument(skip(self))]
    fn get_byte(&mut self) -> Option<u8> {
        self.fill_buffer();

        if self.bit_pos < 8 {
            return None;
        }
        let i = self.bit_buffer;
        self.bit_buffer = self.bit_buffer << 8;
        self.bit_pos -= 8;

        Some(((i & 0xff00) >> 8) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    #[test]
    fn get_bytes() {
        let input: [u8; 6] = [0xFA, 0x50, 0xFF, 0x00, 0x96, 0xC3];
        let mut biterator = Biterator::new(input);

        assert_eq!(biterator.get_byte(), Some(0xFA));
        assert_eq!(biterator.get_byte(), Some(0x50));
        assert_eq!(biterator.get_byte(), Some(0xFF));
        assert_eq!(biterator.get_byte(), Some(0x00));
        assert_eq!(biterator.get_byte(), Some(0x96));
        assert_eq!(biterator.get_byte(), Some(0xC3));
    }

    #[test]
    fn get_bits() {
        let input: [u8; 4] = [0xFA, 0x50, 0x96, 0xC3];
        let mut biterator = Biterator::new(input);

        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(0));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), Some(1));
        dbg!(&biterator);
        assert_eq!(biterator.get_bit(), None);
    }
}

pub struct Encoder<'a> {
    state: LzHufState,
    match_length: u16,
    match_position: u16,
    output: Bitbuffer<'a>,
    lson: [u16; N as usize + 1],
    rson: [u16; N as usize + 257],
    dad: [u16; N as usize + 1],
}

impl<'a> Encoder<'a> {
    fn new(output: &'a mut Vec<u8>) -> Self {
        let mut rson = [0; N as usize + 257];
        for i in (N + 1)..(N + 257) {
            rson[i as usize] = NIL;
        }

        let mut dad = [NIL; N as usize + 1];
        dad[N as usize] = 0;
        Self {
            state: LzHufState::new(),
            match_length: 0,
            match_position: 0,
            output: Bitbuffer::new(output),
            lson: [0; N as usize + 1],
            rson,
            dad,
        }
    }

    fn insert_node(&mut self, r: u16) {
        let mut cmp = 1;
        let key = &self.state.text_buffer[r as usize..];
        let mut p = N + 1 + key[0] as u16;
        self.lson[r as usize] = NIL;
        self.rson[r as usize] = NIL;
        self.match_length = 0;
        loop {
            if cmp >= 0 {
                if self.rson[p as usize] != NIL {
                    p = self.rson[p as usize];
                } else {
                    self.rson[p as usize] = r;
                    self.dad[r as usize] = p;
                    return;
                }
            } else {
                if self.lson[p as usize] != NIL {
                    p = self.lson[p as usize];
                } else {
                    self.lson[p as usize] = r;
                    self.dad[r as usize] = p;
                    return;
                }
            }

            let mut i = 1;
            while i < F {
                cmp = key[i as usize].wrapping_sub(self.state.text_buffer[(p + i) as usize]);
                if cmp != 0 {
                    break;
                }
                i += 1;
            }

            if i > THRESHOLD {
                if i > self.match_length {
                    self.match_position = ((r.wrapping_sub(p)) & (N - 1)) - 1;
                    self.match_length = i;
                    if i >= F {
                        break;
                    }
                }
                if i == self.match_length {
                    let c = ((r.wrapping_sub(p)) & (N - 1)) - 1;
                    if c < self.match_position {
                        self.match_position = c;
                    }
                }
            }
        }

        self.dad[r as usize] = self.dad[p as usize];
        self.lson[r as usize] = self.lson[p as usize];
        self.rson[r as usize] = self.rson[p as usize];
        self.dad[self.lson[p as usize] as usize] = r;
        self.dad[self.rson[p as usize] as usize] = r;

        if self.rson[self.dad[p as usize] as usize] == p {
            self.rson[self.dad[p as usize] as usize] = r;
        } else {
            self.lson[self.dad[p as usize] as usize] = r;
        }

        self.dad[p as usize] = NIL;
    }

    fn delete_node(&mut self, p: u16) {
        if self.dad[p as usize] == NIL {
            return;
        }

        let mut q;
        if self.rson[p as usize] == NIL {
            q = self.lson[p as usize];
        } else if self.lson[p as usize] == NIL {
            q = self.rson[p as usize];
        } else {
            q = self.lson[p as usize];
            if self.rson[q as usize] != NIL {
                loop {
                    q = self.rson[q as usize];
                    if self.rson[q as usize] == NIL {
                        break;
                    }
                }

                self.rson[self.dad[q as usize] as usize] = self.lson[q as usize];
                self.dad[self.lson[q as usize] as usize] = self.dad[q as usize];
                self.lson[q as usize] = self.dad[p as usize];
                self.dad[self.lson[p as usize] as usize] = q;
            }

            self.rson[q as usize] = self.rson[p as usize];
            self.dad[self.rson[p as usize] as usize] = q;
        }

        self.dad[q as usize] = self.dad[p as usize];

        if self.rson[self.dad[p as usize] as usize] == p {
            self.rson[self.dad[p as usize] as usize] = q;
        } else {
            self.lson[self.dad[p as usize] as usize] = q;
        }

        self.dad[p as usize] = NIL;
    }

    fn encode_char(&mut self, c: u16) {
        let mut i = 0u16;
        let mut j = 0u8;
        let mut k = self.state.parents[(c + T) as usize];
        loop {
            i = i >> 1;
            if k & 1 != 0 {
                i = i.wrapping_add(0x8000);
            }
            j += 1;

            k = self.state.parents[k as usize];
            if k == R {
                break;
            }
        }

        self.output.put_code(j, i);
        self.state.update(c as u16);
    }

    fn encode_position(&mut self, c: u16) {
        let i = c >> 6;
        self.output.put_code(p_len[i as usize], (p_code[i as usize] as u16) << 8);
        self.output.put_code(6, (c & 0x3f) << 10);
    }

    fn encode<I: IntoIterator<IntoIter=Y, Item = u8>, Y: Iterator<Item = u8>>(&mut self, input: I) {
        let mut iterator = input.into_iter();
        let mut s = 0;
        let mut r = N - F;
        let mut len = 0;
        while len < F {
            if let Some(b) = iterator.next() {
                self.state.text_buffer[(r + len) as usize] = b;
            } else {
                break;
            }
            len += 1;
        }
        for i in 1..=F {
            self.insert_node(r - i);
        }
        self.insert_node(r);
        loop {
            if self.match_length > len {
                self.match_length = len;
            }
            if self.match_length <= THRESHOLD {
                self.match_length = 1;
                self.encode_char(self.state.text_buffer[r as usize] as u16);
            } else {
                self.encode_char(255 - THRESHOLD + self.match_length);
                self.encode_position(self.match_position);
            }
            let last_match_len = self.match_length;

            let mut i = 0;
            while i < last_match_len {
                if let Some(c) = iterator.next() {
                    self.delete_node(s);
                    self.state.text_buffer[s as usize] = c;
                    if s < F - 1 {
                        self.state.text_buffer[(s + N) as usize] = c;
                    }
                    s = (s + 1) & (N - 1);
                    r = (r + 1) & (N - 1);
                    self.insert_node(r);
                } else {
                    break;
                }
                i = i + 1;
            }

            while i < last_match_len {
                i += 1;
                self.delete_node(s);
                s = (s.wrapping_add(1)) & (N - 1);
                r = (r.wrapping_add(1)) & (N - 1);
                len -= 1;
                if len > 0 {
                    self.insert_node(r)
                }
            }

            if len == 0 {
                break;
            }
        }
    }

    fn finish(self) {}
}

pub struct Decoder<I> {
    state: LzHufState,
    stream: Biterator<I>
}

impl<I> fmt::Debug for Decoder<I> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Decoder")
            .field("state", &self.state)
            .field("stream", &self.stream)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unexpected end of data")]
pub struct UnexpectedEof;

impl<I: Iterator<Item = u8>> Decoder<I> {
    pub fn new<X: IntoIterator<IntoIter = I, Item = u8>>(input: X) -> Self {
        Self {
            state: LzHufState::new(),
            stream: Biterator::new(input),
        }
    }

    #[tracing::instrument(skip(self, buffer))]
    pub fn decode(&mut self, buffer: &mut [u8]) -> Result<(), UnexpectedEof> {
        let mut count = 0;
        while count < buffer.len() {
            let c = self.decode_char().ok_or(UnexpectedEof)?;
            if c < 256 {
                let c = c as u8;
                buffer[count] = c;
                self.state.update_text_buffer(c);
                count += 1;
            } else {
                let i = (self.state.r.wrapping_sub(self.decode_position().ok_or(UnexpectedEof)?).wrapping_sub(1)) & (N - 1);
                let j = c - 255 + THRESHOLD;
                for k in 0..j {
                    let c = self.state.text_buffer[((i + k) & (N - 1)) as usize];
                    buffer[count] = c;
                    self.state.update_text_buffer(c);
                    count += 1;
                }
            }
        }
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    fn decode_char(&mut self) -> Option<u16> {
        let mut c = self.state.children[R as usize];
        while c < T {
            c += self.stream.get_bit()? as u16;
            c = self.state.children[c as usize];
        }
        c -= T;
        self.state.update(c);
        Some(c)
    }

    #[tracing::instrument(skip(self))]
    fn decode_position(&mut self) -> Option<u16> {
        let mut i = self.stream.get_byte()? as u16;
        let c = DECODE_CODE[i as usize] << 6;
        let mut j = DECODE_LEN[i as usize];

        j -= 2;
        for _ in (1..=j).rev() {
            i = (i << 1) + self.stream.get_bit()? as u16;
        }
        Some(c as u16 | (i & 0x3f))
    }
}

const p_len: [u8; 64] = [
    0x03, 0x04, 0x04, 0x04, 0x05, 0x05, 0x05, 0x05,
    0x05, 0x05, 0x05, 0x05, 0x06, 0x06, 0x06, 0x06,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08,
    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08,
];

const p_code: [u8; 64] = [
    0x00, 0x20, 0x30, 0x40, 0x50, 0x58, 0x60, 0x68,
    0x70, 0x78, 0x80, 0x88, 0x90, 0x94, 0x98, 0x9C,
    0xA0, 0xA4, 0xA8, 0xAC, 0xB0, 0xB4, 0xB8, 0xBC,
    0xC0, 0xC2, 0xC4, 0xC6, 0xC8, 0xCA, 0xCC, 0xCE,
    0xD0, 0xD2, 0xD4, 0xD6, 0xD8, 0xDA, 0xDC, 0xDE,
    0xE0, 0xE2, 0xE4, 0xE6, 0xE8, 0xEA, 0xEC, 0xEE,
    0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7,
    0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD, 0xFE, 0xFF,
];

const DECODE_CODE: [u8; 256] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
    0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02,
    0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02,
    0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
    0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
    0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08,
    0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09,
    0x0A, 0x0A, 0x0A, 0x0A, 0x0A, 0x0A, 0x0A, 0x0A,
    0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B,
    0x0C, 0x0C, 0x0C, 0x0C, 0x0D, 0x0D, 0x0D, 0x0D,
    0x0E, 0x0E, 0x0E, 0x0E, 0x0F, 0x0F, 0x0F, 0x0F,
    0x10, 0x10, 0x10, 0x10, 0x11, 0x11, 0x11, 0x11,
    0x12, 0x12, 0x12, 0x12, 0x13, 0x13, 0x13, 0x13,
    0x14, 0x14, 0x14, 0x14, 0x15, 0x15, 0x15, 0x15,
    0x16, 0x16, 0x16, 0x16, 0x17, 0x17, 0x17, 0x17,
    0x18, 0x18, 0x19, 0x19, 0x1A, 0x1A, 0x1B, 0x1B,
    0x1C, 0x1C, 0x1D, 0x1D, 0x1E, 0x1E, 0x1F, 0x1F,
    0x20, 0x20, 0x21, 0x21, 0x22, 0x22, 0x23, 0x23,
    0x24, 0x24, 0x25, 0x25, 0x26, 0x26, 0x27, 0x27,
    0x28, 0x28, 0x29, 0x29, 0x2A, 0x2A, 0x2B, 0x2B,
    0x2C, 0x2C, 0x2D, 0x2D, 0x2E, 0x2E, 0x2F, 0x2F,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37,
    0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F,
];

const DECODE_LEN: [u8; 256] = [
    0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
    0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
    0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
    0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
    0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08,
    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08,
];

#[cfg(test)]
mod tests2 {
    use super::*;
    use test_log::test;

    #[test]
    fn test_decode() -> color_eyre::Result<()> {
        let input = &include_bytes!("../samples/winlink.raw")[0x2F..0x10C];
        let mut decoder = Decoder::new(input.iter().copied());
        let data_spot = &mut [0u8; 0x123];
        decoder.decode(data_spot)?;
        let data = std::str::from_utf8(data_spot).unwrap();
        assert_eq!(data, include_str!("../samples/winlink.txt"));
        Ok(())
    }

    #[test]
    fn test_encode() -> color_eyre::Result<()> {
        let input: &str = include_str!("../samples/winlink.txt");
        let mut output = Vec::with_capacity(input.len());
        let mut encoder = Encoder::new(&mut output);
        encoder.encode(input.as_bytes().iter().copied());
        encoder.finish();
        assert_eq!(&output, &include_bytes!("../samples/winlink.raw")[0x2F..0x10C]);
        Ok(())
    }

    #[test]
    fn test_decode_single_byte() -> color_eyre::Result<()> {
        let input: [u8; 2] = [0xEC, 0x80];
        let mut decoder = Decoder::new(input);
        let data_spot = &mut [0u8; 1];
        decoder.decode(data_spot)?;
        assert_eq!(*data_spot, [0x4D]);
        Ok(())
    }

    #[test]
    fn test_encode_single_byte() -> color_eyre::Result<()> {
        let input: [u8; 1] = [0x4D];
        let mut output = Vec::with_capacity(input.len());
        let mut encoder = Encoder::new(&mut output);
        encoder.encode(input);
        encoder.finish();
        assert_eq!(&output, &[0xEC, 0x80]);
        Ok(())
    }

    #[test]
    fn test_decode_two_bytes() -> color_eyre::Result<()> {
        let input: [u8; 3] = [0xEC, 0xE2, 0x80];
        let mut decoder = Decoder::new(input);
        let data_spot = &mut [0u8; 2];
        decoder.decode(data_spot)?;
        assert_eq!(*data_spot, [0x4D, 0x4D]);
        Ok(())
    }

    #[test]
    fn test_encode_two_byte() -> color_eyre::Result<()> {
        let input: [u8; 2] = [0x4D, 0x4D];
        let mut output = Vec::with_capacity(input.len());
        let mut encoder = Encoder::new(&mut output);
        encoder.encode(input);
        encoder.finish();
        assert_eq!(&output, &[0xEC, 0xE2, 0x80]);
        Ok(())
    }

    #[test]
    fn test_decode_thirty_two_bytes() -> color_eyre::Result<()> {
        let input: [u8; 4] = [0xEC, 0xD4, 0x00, 0x00];
        let mut decoder = Decoder::new(input);
        let data_spot = &mut [0u8; 32];
        decoder.decode(data_spot)?;
        assert_eq!(*data_spot, [0x4D; 32]);
        Ok(())
    }

    #[test]
    fn test_encode_thirty_two_byte() -> color_eyre::Result<()> {
        let input: [u8; 32] = [0x4D; 32];
        let mut output = Vec::with_capacity(input.len());
        let mut encoder = Encoder::new(&mut output);
        encoder.encode(input);
        encoder.finish();
        assert_eq!(&output, &[0xEC, 0xD4, 0x00, 0x00]);
        Ok(())
    }
}
