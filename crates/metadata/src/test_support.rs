//! What more than one module's tests use: a PNG chunk with its CRC.

pub(crate) fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
    c.extend(kind);
    c.extend(data);
    c.extend(crc32fast::hash(&[&kind[..], data].concat()).to_be_bytes());
    c
}
