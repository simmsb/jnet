//! UDP: User Datagram Protocol

use core::ops::{Range, RangeFrom};
use core::{fmt, u16};

use as_slice::{AsMutSlice, AsSlice};
use byteorder::{ByteOrder, NetworkEndian as NE};
use cast::{u16, usize};
use owning_slice::Truncate;

use crate::{
    coap::{self, Unset},
    ipv6,
    traits::UncheckedIndex,
};

/* Packet structure */
const SOURCE: Range<usize> = 0..2;
const DESTINATION: Range<usize> = 2..4;
const LENGTH: Range<usize> = 4..6;
const CHECKSUM: Range<usize> = 6..8;
const PAYLOAD: RangeFrom<usize> = 8..;

/// Size of the UDP header
pub const HEADER_SIZE: u8 = PAYLOAD.start as u8;

/// UDP packet
pub struct Packet<BUFFER>
where
    BUFFER: AsSlice<Element = u8>,
{
    buffer: BUFFER,
}

impl<B> Packet<B>
where
    B: AsSlice<Element = u8>,
{
    /* Constructors */
    /// Parses the bytes as an UDP packet
    pub fn parse(bytes: B) -> Result<Self, B> {
        let nbytes = bytes.as_slice().len();
        if nbytes < usize(HEADER_SIZE) {
            return Err(bytes);
        }

        let packet = Packet { buffer: bytes };
        let len = packet.get_length();

        if len < u16(HEADER_SIZE) || usize(len) > nbytes {
            Err(packet.buffer)
        } else {
            Ok(packet)
        }
    }

    /* Getters */
    /// Returns the Source (port) field of the header
    pub fn get_source(&self) -> u16 {
        NE::read_u16(&self.header_()[SOURCE])
    }

    /// Returns the Destination (port) field of the header
    pub fn get_destination(&self) -> u16 {
        NE::read_u16(&self.header_()[DESTINATION])
    }

    /// Returns the Length field of the header
    pub fn get_length(&self) -> u16 {
        NE::read_u16(&self.header_()[LENGTH])
    }

    /// get the udp checksum
    pub fn get_checksum(&self) -> u16 {
        NE::read_u16(&self.header_()[CHECKSUM])
    }

    /// Returns the length (header + data) of this packet
    pub fn len(&self) -> u16 {
        self.get_length()
    }

    /* Miscellaneous */
    /// View into the payload
    pub fn payload(&self) -> &[u8] {
        unsafe { self.as_slice().rf(PAYLOAD) }
    }

    /// Returns the byte representation of this UDP packet
    pub fn as_bytes(&self) -> &[u8] {
        self.as_slice()
    }

    /* Miscellaneous */
    pub(crate) fn compute_checksum(&self, src: ipv6::Addr, dest: ipv6::Addr) -> u16 {
        const NEXT_HEADER: u8 = 17;

        let mut sum: u32 = 0;

        // Pseudo-header
        for chunk in src.0.chunks_exact(2).chain(dest.0.chunks_exact(2)) {
            sum += u32::from(NE::read_u16(chunk));
        }

        // XXX should this be just `as u16`?
        let len = self.as_slice().len() as u32;
        sum += len >> 16;
        sum += len & 0xffff;

        sum += u32::from(NEXT_HEADER);

        // UDP message
        for (i, chunk) in self.as_slice().chunks(2).enumerate() {
            if i == 3 {
                // this is the checksum field, skip
                continue;
            }

            if chunk.len() == 1 {
                sum += u32::from(chunk[0]) << 8;
            } else {
                sum += u32::from(NE::read_u16(chunk));
            }
        }

        // fold carry-over
        while sum >> 16 != 0 {
            sum = (sum & 0xffff) + (sum >> 16);
        }

        !(sum as u16)
    }

    /// Verifies the 'Checksum' field
    pub fn verify_ipv6_checksum(&self, src: ipv6::Addr, dest: ipv6::Addr) -> bool {
        self.compute_checksum(src, dest) == self.get_checksum()
    }

    /* Private */
    fn as_slice(&self) -> &[u8] {
        self.buffer.as_slice()
    }

    fn header_(&self) -> &[u8; HEADER_SIZE as usize] {
        debug_assert!(self.as_slice().len() >= HEADER_SIZE as usize);

        unsafe { &*(self.as_slice().as_ptr() as *const _) }
    }

    fn payload_len(&self) -> u16 {
        self.get_length() - u16(HEADER_SIZE)
    }
}

impl<B> Packet<B>
where
    B: AsSlice<Element = u8> + AsMutSlice<Element = u8>,
{
    /* Setters */
    /// Sets the Source (port) field of the header
    pub fn set_source(&mut self, port: u16) {
        NE::write_u16(&mut self.header_mut_()[SOURCE], port)
    }

    /// Sets the Destination (port) field of the header
    pub fn set_destination(&mut self, port: u16) {
        NE::write_u16(&mut self.header_mut_()[DESTINATION], port)
    }

    unsafe fn set_length(&mut self, len: u16) {
        NE::write_u16(&mut self.header_mut_()[LENGTH], len)
    }

    /// Zeroes the Checksum field of the header
    pub fn zero_checksum(&mut self) {
        self.set_checksum(0);
    }

    /// Sets the Destination (port) field of the header
    fn set_checksum(&mut self, checksum: u16) {
        NE::write_u16(&mut self.header_mut_()[CHECKSUM], checksum)
    }

    /* Miscellaneous */
    /// Mutable view into the payload
    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.as_mut_slice()[PAYLOAD]
    }

    /// Recomputes and updates the 'Checksum' field
    pub fn update_ipv6_checksum(&mut self, src: ipv6::Addr, dest: ipv6::Addr) {
        let cksum = self.compute_checksum(src, dest);
        self.set_checksum(cksum)
    }

    /* Private */
    fn as_mut_slice(&mut self) -> &mut [u8] {
        self.buffer.as_mut_slice()
    }

    fn header_mut_(&mut self) -> &mut [u8; HEADER_SIZE as usize] {
        debug_assert!(self.as_slice().len() >= HEADER_SIZE as usize);

        unsafe { &mut *(self.as_mut_slice().as_mut_ptr() as *mut _) }
    }
}

impl<B> Packet<B>
where
    B: AsSlice<Element = u8> + AsMutSlice<Element = u8> + Truncate<u16>,
{
    /* Constructors */
    /// Transforms the given buffer into an UDP packet
    ///
    /// NOTE The UDP packet will span the whole buffer and the Checksum field will be zeroed.
    ///
    /// # Panics
    ///
    /// This constructor panics if the given `buffer` is not large enough to contain the UDP header.
    pub fn new(mut buffer: B) -> Option<Self> {
        if buffer.as_slice().len() < usize(HEADER_SIZE) {
            return None;
        }

        let len = u16(buffer.as_slice().len()).unwrap_or(u16::MAX);
        buffer.truncate(len);
        let mut packet = Packet { buffer };

        packet.set_checksum(0);
        unsafe { packet.set_length(len) }

        Some(packet)
    }

    /* Setters */
    /// Fills the payload with the given data and adjusts the length of the UDP packet
    pub fn set_payload(&mut self, data: &[u8]) -> Option<()> {
        let len = u16(data.len()).unwrap();
        if self.payload_len() < len {
            return None;
        }

        self.truncate(len);
        self.payload_mut().copy_from_slice(data);

        Some(())
    }

    /* Miscellaneous */
    /// Fills the payload with a CoAP message
    pub fn coap<F>(&mut self, token_length: u8, f: F)
    where
        F: FnOnce(coap::Message<&mut [u8], Unset>) -> coap::Message<&mut [u8]>,
    {
        let len = {
            let m = coap::Message::new(self.payload_mut(), token_length);
            f(m).len()
        };
        self.truncate(len);
    }

    /// Truncates the *payload* to the specified length
    pub fn truncate(&mut self, len: u16) {
        if len < self.payload_len() {
            let total_len = len + u16(HEADER_SIZE);
            self.buffer.truncate(total_len);
            unsafe { self.set_length(total_len) }
        }
    }
}

/// NOTE excludes the payload
impl<B> fmt::Debug for Packet<B>
where
    B: AsSlice<Element = u8>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("udp::Packet")
            .field("source", &self.get_source())
            .field("destination", &self.get_destination())
            .field("length", &self.get_length())
            .field("checksum", &self.get_checksum())
            // .field("payload", &self.payload())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use cast::u16;
    use rand::{self, RngCore};

    use crate::{ether, ipv4, mac, udp};

    const SIZE: usize = 56;

    const BYTES: &[u8; SIZE] = &[
        255, 255, 255, 255, 255, 255, // ether: destination
        1, 1, 1, 1, 1, 1, // ether: source
        8, 0,  // ether: type
        69, // ipv4: version & IHL
        0,  // ipv4: DSCP & ECN
        0, 42, //ipv4: total length
        0, 0, // ipv4: identification
        64, 0,  // ipv4: fragment
        64, //ipv4: ttl
        17, //ipv4: protocol
        185, 80, // ipv4: checksum
        192, 168, 0, 33, // ipv4: source
        192, 168, 0, 1, // ipv4: destination
        0, 0, // udp: source
        5, 57, // udp: destination
        0, 22, // udp: length
        0, 0, // udp: checksum
        72, 101, 108, 108, 111, 44, 32, 119, 111, 114, 108, 100, 33, 10, // udp: payload
    ];

    const MAC_SRC: mac::Addr = mac::Addr([0x01; 6]);
    const MAC_DST: mac::Addr = mac::Addr([0xff; 6]);

    const IP_SRC: ipv4::Addr = ipv4::Addr([192, 168, 0, 33]);
    const IP_DST: ipv4::Addr = ipv4::Addr([192, 168, 0, 1]);

    const UDP_DST: u16 = 1337;

    const MESSAGE: &[u8] = b"Hello, world!\n";

    #[test]
    fn construct() {
        // NOTE start with randomized array to make sure we set *everything* correctly
        let mut array: [u8; SIZE] = [0; SIZE];
        rand::thread_rng().fill_bytes(&mut array);

        let mut eth = ether::Frame::new(&mut array[..]);

        eth.set_destination(MAC_DST);
        eth.set_source(MAC_SRC);

        eth.ipv4(|ip| {
            ip.set_destination(IP_DST);
            ip.set_source(IP_SRC);

            ip.udp(|udp| {
                udp.set_source(0);
                udp.set_destination(UDP_DST);
                udp.set_payload(MESSAGE);
            });
        });

        assert_eq!(eth.as_bytes(), &BYTES[..]);
    }

    #[test]
    fn new() {
        const SZ: u16 = 128;

        let mut chunk = [0; SZ as usize];
        let buf = &mut chunk[..];

        let udp = udp::Packet::new(buf);
        assert_eq!(udp.len(), SZ);
        assert_eq!(udp.get_length(), SZ);
    }

    #[test]
    fn parse() {
        let eth = ether::Frame::parse(&BYTES[..]).unwrap();
        assert_eq!(eth.get_destination(), MAC_DST);
        assert_eq!(eth.get_source(), MAC_SRC);
        assert_eq!(eth.get_type(), ether::Type::Ipv4);

        let ip = ipv4::Packet::parse(eth.payload()).unwrap();
        assert_eq!(ip.get_source(), IP_SRC);
        assert_eq!(ip.get_destination(), IP_DST);

        let udp = udp::Packet::parse(ip.payload()).unwrap();
        assert_eq!(udp.get_source(), 0);
        assert_eq!(udp.get_destination(), UDP_DST);
        assert_eq!(
            udp.get_length(),
            MESSAGE.len() as u16 + u16(udp::HEADER_SIZE)
        );
        assert_eq!(udp.payload(), MESSAGE);
    }
}
