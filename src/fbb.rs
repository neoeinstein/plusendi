use std::fmt;
use std::fmt::Formatter;
use aliri_braid::braid;
use nom::branch::alt;
use nom::{IResult, Parser};
use nom::bytes::complete::take_while1;
use nom::bytes::streaming::{tag, take, take_until1, take_while_m_n};
use nom::character::is_digit;
use nom::combinator::{all_consuming, map, map_res, not, opt, value, verify};
use nom::error::VerboseError;
use nom::number::streaming::be_u8;
use nom::sequence::{delimited, preceded, separated_pair, terminated, tuple};
use crate::crc16::Crc16;
use crate::lzhuf::Decoder;
use crate::{StationId, StationIdRef};

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
    fn decompress(self) -> Result<Vec<u8>, crate::lzhuf::UnexpectedEof> {
        let mut buffer = vec![0; self.uncompressed_size as usize];
        let mut decoder = Decoder::new(self.blocks.into_iter().flatten().copied());
        decoder.decode(&mut buffer)?;
        Ok(buffer)
    }
}

fn b2_message_block(data: &[u8]) -> IResult<&[u8], CompressedMessage, VerboseError<&[u8]>> {
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

fn fa_tag(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    tag("FA")(data)
}

fn fb_tag(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    tag("FB")(data)
}

fn fc_tag(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    tag("FC")(data)
}

fn end_of_proposal_tag(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    tag("F>")(data)
}

fn select_tag(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    tag("FS")(data)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageChoice {
    Accept { offset: u16 },
    Defer,
    Reject,
}

fn delimiter(c: u8) -> bool {
    c == b' ' || c == b'\r'
}

fn selection_element(data: &[u8]) -> IResult<&[u8], MessageChoice, VerboseError<&[u8]>> {
    alt((
        value(MessageChoice::Accept { offset: 0 }, alt((tag("+"), tag("Y"), tag("H")))),
        map(preceded(
            alt((tag("!"), tag("A"))),
            map_res(map_res(take_while_m_n(1, 6, is_digit), std::str::from_utf8), |s| u16::from_str_radix(s, 10)),
        ), |offset| MessageChoice::Accept { offset }),
        value(MessageChoice::Defer, alt((tag("="), tag("L")))),
        value(MessageChoice::Reject, alt((tag("-"), tag("N"), tag("R"), tag("E")))),
    ))(data)
}

fn selection<const P: usize>(data: &[u8]) -> IResult<&[u8], [MessageChoice; P], VerboseError<&[u8]>> {
    let (mut data, _) = terminated(select_tag, tag(" "))(data)?;
    let mut responses = [MessageChoice::Defer; P];
    for response in &mut responses {
        let (rem, choice) = selection_element(data)?;
        data = rem;
        *response = choice;
    }
    Ok((data, responses))
}

fn no_more(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    tag("FF")(data)
}

fn all_done(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    tag("FQ")(data)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    Private,
    Bulletin,
    Traffic,
}

fn message_type(data: &[u8]) -> IResult<&[u8], MessageType, VerboseError<&[u8]>> {
    alt((
        tag("P").map(|_| MessageType::Private),
        tag("B").map(|_| MessageType::Bulletin),
        tag("T").map(|_| MessageType::Traffic),
    ))(data)
}

#[braid]
struct Sender;

fn sender(data: &[u8]) -> IResult<&[u8], &SenderRef, VerboseError<&[u8]>> {
    map(map_res(take_while_m_n(1, 6, |x| !delimiter(x)), std::str::from_utf8), SenderRef::from_str)(data)
}

#[braid]
struct Recipient;

#[braid]
struct AtBbs;

struct Addressee<'a> {
    recipient: &'a RecipientRef,
    mbo: &'a AtBbsRef,
}

fn recipient(data: &[u8]) -> IResult<&[u8], Addressee, VerboseError<&[u8]>> {
    map(separated_pair(
        map(map_res(take_while_m_n(1, 40, |x| !delimiter(x)), std::str::from_utf8), AtBbsRef::from_str),
            tag(" "),
        map(map_res(take_while_m_n(1, 6, |x| !delimiter(x)), std::str::from_utf8), RecipientRef::from_str),
    ), |(mbo, recipient)| Addressee { recipient, mbo })(data)
}

#[braid]
struct MessageId;

fn message_id(data: &[u8]) -> IResult<&[u8], &MessageIdRef, VerboseError<&[u8]>> {
    map(map_res(take_while_m_n(1, 12, |x| !delimiter(x)), std::str::from_utf8), MessageIdRef::from_str)(data)
}

fn message_size(data: &[u8]) -> IResult<&[u8], u16, VerboseError<&[u8]>> {
    map_res(map_res(take_while_m_n(1, 6, is_digit), std::str::from_utf8), |s| u16::from_str_radix(s, 10))(data)
}

fn fbb_proposal(data: &[u8]) -> IResult<&[u8], Proposal, VerboseError<&[u8]>> {
    delimited(
        alt((fa_tag, fb_tag)),
        tuple((
            preceded(tag(" "), message_type),
            preceded(tag(" "), sender),
            preceded(tag(" "), recipient),
            preceded(tag(" "), message_id),
            preceded(tag(" "), message_size),
        )).map(Proposal::from_parts),
        tag("\r"),
    )(data)
}

struct Proposal<'a> {
    message_type: MessageType,
    sender: &'a SenderRef,
    addressee: Addressee<'a>,
    message_id: &'a MessageIdRef,
    message_size: u16,
}

impl<'a> Proposal<'a> {
    fn from_parts((message_type, sender, addressee, message_id, message_size): (MessageType, &'a SenderRef, Addressee<'a>, &'a MessageIdRef, u16)) -> Self {
        Self {
            message_type,
            sender,
            addressee,
            message_id,
            message_size,
        }
    }
}

fn winlink_proposal(data: &[u8]) -> IResult<&[u8], WinlinkProposal, VerboseError<&[u8]>> {
    delimited(
        fc_tag,
        map(tuple((
            preceded(tag(" "), tag("EM")),
            preceded(tag(" "), message_id),
            preceded(tag(" "), message_size),
            preceded(tag(" "), message_size),
            opt(
                map(tuple((
                    preceded(tag(" "), sender),
                    preceded(tag(" "), recipient),
                )), BqpProposalExtension::from_parts)
            )
        )), WinlinkProposal::from_parts),
        tag("\r")
    )(data)
}

struct WinlinkProposal<'a> {
    message_id: &'a MessageIdRef,
    compressed_message_size: u16,
    uncompressed_message_size: u16,
    bqp_extension: Option<BqpProposalExtension<'a>>
}

impl<'a> WinlinkProposal<'a> {
    fn from_parts((_, message_id, uncompressed_message_size, compressed_message_size, bqp_extension): (&[u8], &'a MessageIdRef, u16, u16, Option<BqpProposalExtension<'a>>)) -> Self {
        Self {
            message_id,
            uncompressed_message_size,
            compressed_message_size,
            bqp_extension,
        }
    }
}

struct BqpProposalExtension<'a> {
    sender: &'a SenderRef,
    addressee: Addressee<'a>,
}

impl<'a> BqpProposalExtension<'a> {
    fn from_parts((sender, addressee): (&'a SenderRef, Addressee<'a>)) -> Self {
        Self {
            sender,
            addressee,
        }
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;
    use super::*;

    #[test]
    fn try_it_out() -> color_eyre::Result<()> {
        let input = include_bytes!("../samples/winlink.raw");
        let (_, data) = all_consuming(b2_message_block)(&input[..])?;
        data.decompress();
        Ok(())
        // Err(color_eyre::eyre::eyre!("just need a forced failure"))
    }

    #[test]
    fn try_it_out2() -> color_eyre::Result<()> {
        let input = include_bytes!("../samples/winlink2.raw");
        let (_, data) = all_consuming(b2_message_block)(&input[..])?;
        println!("{}", crate::parser::StrOrByteSlice::Bytes(&data.decompress().unwrap()));
        Ok(())
        // Err(color_eyre::eyre::eyre!("just need a forced failure"))
    }

    #[test]
    fn partial_b2_message_block_in_middle_of_block() -> color_eyre::Result<()> {
        let input = include_bytes!("../samples/packet1.raw");
        let result = all_consuming(b2_message_block)(&input[..]);
        match result {
            Err(nom::Err::Incomplete(nom::Needed::Size(amt))) if amt.get() == 0x90 => Ok(()),
            Err(nom::Err::Incomplete(nom::Needed::Size(amt))) => panic!("expected incomplete with at least 144 more bytes required, but need a known {} bytes", amt),
            Err(nom::Err::Incomplete(nom::Needed::Unknown)) => panic!("expected incomplete with at least 144 more bytes required, but need an unknown number of bytes"),
            Err(err) => Err(err.into()),
            Ok(wat) => panic!("this should not have succeeded"),
        }
        // Err(color_eyre::eyre::eyre!("just need a forced failure"))
    }

    #[test]
    fn partial_b2_message_block_on_block_boundary() -> color_eyre::Result<()> {
        let input1 = include_bytes!("../samples/packet1.raw");
        let input2 = include_bytes!("../samples/packet2.raw");
        let input: Vec<u8> = input1.iter().chain(input2.iter().take(0x90)).copied().collect();
        let result = all_consuming(b2_message_block)(&input[..]);
        match result {
            Err(nom::Err::Incomplete(nom::Needed::Size(amt))) if amt.get() == 0x01 => Ok(()),
            Err(nom::Err::Incomplete(nom::Needed::Size(amt))) => panic!("expected incomplete with at least 1 more byte required, but need a known {} bytes", amt),
            Err(nom::Err::Incomplete(nom::Needed::Unknown)) => panic!("expected incomplete with at least 1 more byte required, but need an unknown number of bytes"),
            Err(err) => panic!("Received some other weird error: {}", err),
            Ok(wat) => panic!("this should not have succeeded"),
        }
        // Err(color_eyre::eyre::eyre!("just need a forced failure"))
    }

    // #[test]
    // fn compress() -> color_eyre::Result<()> {
    //     let input = include_bytes!("../samples/winlink.txt");
    //     let mut encoder = compression::prelude::LzhufEncoder::new(&compression::prelude::LzhufMethod::Lh4);
    //     let result: Result<Vec<u8>, _> = (&input[..]).iter().copied().encode(&mut encoder, compression::prelude::Action::Finish).collect();
    //     let result = result?;
    //
    //     dbg!(result);
    //     Err(color_eyre::eyre::eyre!("just need a forced failure"))
    // }
}
