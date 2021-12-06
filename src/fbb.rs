use nom::IResult;
use nom::bytes::streaming::{tag, take};
use nom::character::is_digit;
use nom::combinator::{all_consuming, verify};
use nom::error::VerboseError;
use nom::number::streaming::be_u8;
use nom::sequence::{preceded, separated_pair, terminated};
use crate::crc16::Crc16;
use crate::lzhuf::Decoder;

fn soh(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    tag(&[0x01])(data)
}

fn nul(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    tag(&[0x00])(data)
}

fn stx(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    tag(&[0x02])(data)
}

fn eot(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    tag(&[0x04])(data)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompressedMessage<'a> {
    title: &'a str,
    offset: u32,
    crc16: u16,
    uncompressed_size: u32,
    blocks: Vec<&'a [u8]>,
}

impl<'a> CompressedMessage<'a> {
    fn decompress(self) -> Result<Vec<u8>, &'static str> {
        println!("Title: {}", self.title);
        println!("CRC16: {:#0X}", self.crc16);
        println!("Uncompressed Size: {}", self.uncompressed_size);

        let mut buffer = vec![0; self.uncompressed_size as usize];
        let mut decoder = Decoder::new(self.blocks.into_iter().flatten().copied());
        decoder.decode(&mut buffer)?;

        println!("Data: {}", String::from_utf8_lossy(&buffer));
        Ok(buffer)
    }
}

fn binary_compressed_v1(data: &[u8]) -> IResult<&[u8], CompressedMessage, VerboseError<&[u8]>> {
    let (rest, (title, offset)) = header(data)?;
    let (rest, (crc16, uncompressed_size, blocks, _)) =
        verify(
            data_blocks,
            |(crc16, uncompressed_size, blocks, checksum)| {
                // println!("checksum: {:#0X}", checksum); //A1
                // println!("just blocks: {:#0X}", (&blocks[..]).iter().copied().flatten().fold(0u8, |x, &y| x.wrapping_add(y)));
                // println!("header bytes: {:#0X}", h_bytes.iter().fold(0u8, |x, &y| x.wrapping_add(y)));
                // println!("all bytes: {:#0X}", data.iter().rev().skip(2).fold(0u8, |x, &y| x.wrapping_add(y)));
                // println!("total: {:#0X}", bytes.iter().rev().skip(2).fold(0u8, |x, &y| x.wrapping_add(y)));
                let check =
                    ((crc16.wrapping_add(crc16 >> 8)) as u32)
                        .wrapping_add(
                            uncompressed_size
                                .wrapping_add(uncompressed_size >> 8)
                                .wrapping_add(uncompressed_size >> 16)
                                .wrapping_add(uncompressed_size >> 24)
                        );
                let check = blocks[..].iter().copied().flatten().copied().fold((check & 0xff) as u8, |x, y| x.wrapping_add(y));
                let checksum_ok = checksum.wrapping_add(check) == 0;

                if !checksum_ok {
                    tracing::warn!(sum = format!("{:#04X}", check).as_str(), expected = format!("{:#04X}", checksum).as_str(), actual = format!("{:#04X}", -(check as i8)).as_str(), "checksum mismatch");
                }

                let mut crc = Crc16::new();
                uncompressed_size.to_le_bytes().into_iter().for_each(|b| crc.update(b));
                blocks[..].iter().copied().flatten().copied().for_each(|b| crc.update(b));
                let actual_crc = crc.finish();
                let crc16_ok = *crc16 == actual_crc;

                if !crc16_ok {
                    tracing::warn!(expected = format!("{:#06X}", crc16).as_str(), actual = format!("{:#06X}", actual_crc).as_str(), "CRC16 mismatch");
                }

                checksum_ok && crc16_ok
            }
        )(rest)?;

    Ok((rest, CompressedMessage {
        title,
        offset,
        crc16,
        uncompressed_size,
        blocks,
    }))
}

fn header(data: &[u8]) -> IResult<&[u8], (&str, u32), VerboseError<&[u8]>> {
    let (rest, data) = preceded(soh, verify(be_u8, |&v| v <= 88))(data)?;
    let (rest, header) = take(data)(rest)?;
    let (_, (title, offset)) = all_consuming(terminated(
        separated_pair(
            nom::combinator::map(
                nom::bytes::complete::take_while(|c: u8| c.is_ascii_graphic() || c == b' '),
                |bytes: &[u8]| unsafe { std::str::from_utf8_unchecked(bytes) }
            ),
            nul,
            nom::combinator::map_res(
                nom::bytes::complete::take_while(is_digit),
                |bytes: &[u8]| {
                    u32::from_str_radix(unsafe { std::str::from_utf8_unchecked(bytes) }, 10)
                }),
        ),
        nul
    ))(header)?;
    Ok((rest, (title, offset)))
}

fn checksum(data: &[u8]) -> IResult<&[u8], u8, VerboseError<&[u8]>> {
    preceded(eot, be_u8)(data)
}

fn data_blocks(data: &[u8]) -> IResult<&[u8], (u16, u32, Vec<&[u8]>, u8), VerboseError<&[u8]>> {
    let (rest, (mut blocks, checksum)) = nom::multi::many_till(data_block, checksum)(data)?;

    let (crc16checksum, uncompressed_size) = if let Some(block) = blocks.get_mut(0) {
        let (data, (crc16checksum, uncompressed_size)) = first_data_block(&*block)?;
        *block = data;
        (crc16checksum, uncompressed_size)
    } else {
        (0, 0)
    };

    Ok((
        rest,
        (crc16checksum, uncompressed_size, blocks, checksum)
    ))
}

fn first_data_block(data: &[u8]) -> IResult<&[u8], (u16, u32), VerboseError<&[u8]>> {
    // let (rest, data) = data_block(data)?;
    let (data, checksum) = nom::number::complete::le_u16(data)?;
    let (data, uncompressed_size) = nom::number::complete::le_u32(data)?;
    Ok((data, (checksum, uncompressed_size)))
}

fn data_block(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    let (rest, len) = preceded(stx, be_u8)(data)?;
    let count = if len == 0 { 256 } else { len as usize };
    take(count)(rest)
}

#[cfg(test)]
mod tests {
    use test_log::test;
    use nom::combinator::all_consuming;
    use crate::fbb::binary_compressed_v1;

    #[test]
    fn try_it_out() -> color_eyre::Result<()> {
        let input = include_bytes!("../samples/winlink.raw");
        let (_, data) = all_consuming(binary_compressed_v1)(&input[..])?;
        data.decompress();

        Err(color_eyre::eyre::eyre!("just need a forced failure"))
    }

    #[test]
    fn compress() -> color_eyre::Result<()> {
        use compression::prelude::EncodeExt;
        let input = include_bytes!("../samples/winlink.txt");
        let mut encoder = compression::prelude::LzhufEncoder::new(&compression::prelude::LzhufMethod::Lh4);
        let result: Result<Vec<u8>, _> = (&input[..]).iter().copied().encode(&mut encoder, compression::prelude::Action::Finish).collect();
        let result = result?;

        dbg!(result);
        Err(color_eyre::eyre::eyre!("just need a forced failure"))
    }
}
