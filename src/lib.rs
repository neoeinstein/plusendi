use std::collections::hash_set::HashSet;

pub mod fbb;
pub mod modem;
pub mod rig;
mod crc16;
mod lzhuf;
mod types;
mod parser;

pub use modem::Modem;
pub use types::{StationId, StationIdRef};

#[derive(Debug, PartialEq, Eq)]
pub struct Traffic {
    pub header: TrafficHeader,
    pub destination: Destination,
    pub body: String,
    pub signature: Signature,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Destination {
    pub addressee: String,
    pub station: Option<StationId>,
    pub address: Vec<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub op_note: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Signature {
    pub signed_by: String,
    pub op_note: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TrafficHeader {
    pub service: ServiceType,
    pub number: u16,
    pub traffic_type: TrafficType,
    pub precedence: Precedence,
    pub handling: Handling,
    pub originator: StationId,
    pub check: Check,
    pub origin: String,
    pub time_filed: Option<String>,
    pub date: String,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ServiceType {
    Normal,
    Service,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum TrafficType {
    Normal,
    Test,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Precedence {
    Routine,
    Welfare,
    Priority,
    Emergency,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Handling {
    directives: HashSet<HandlingDirective>,
}

impl Handling {
    fn with_directives<I: IntoIterator<IntoIter=J, Item=HandlingDirective>, J: Iterator<Item=HandlingDirective>>(directives: I) -> Self {
        Handling {
            directives: directives.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum HandlingDirective {
    LandlineCollect {
        distance: u16,
    },
    DeliverWithin {
        hours: u8,
    },
    ReportDelivery,
    TraceRelayAndDelivery,
    RequestReply,
    HoldUntil {
        date: String,
    },
    CancelIfFeeRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    content: ContentType,
    count: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentType {
    Standard,
    Arl,
}

fn test() -> Traffic {
    Traffic {
        header: TrafficHeader {
            number: 22,
            traffic_type: TrafficType::Normal,
            precedence: Precedence::Routine,
            handling: Handling::with_directives([
                HandlingDirective::ReportDelivery,
            ]),
            check: Check {
                content: ContentType::Standard,
                count: 21,
            },
            originator: StationId::new("KC1GSL").unwrap(),
            origin: String::from("BILLERICA MA"),
            service: ServiceType::Normal,
            time_filed: None,
            date: String::from("DEC 3"),
        },
        destination: Destination {
            addressee: String::from("BOB SPARKES"),
            station: Some(StationId::new("KC1KVY").unwrap()),
            address: Vec::new(),
            phone: None,
            email: None,
            op_note: None,
        },
        signature: Signature {
            signed_by: String::from("MARCUS KC1GSL"),
            op_note: None,
        },
        body: String::from("THIS IS A TEST OF A PROGRAM I WROTE TO ASSIST ME IN PUSHING TRAFFIC INTO THE DIGITAL TRAFFIC NETWORK 73"),
    }
}
