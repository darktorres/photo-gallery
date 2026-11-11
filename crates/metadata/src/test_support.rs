//! What more than one module's tests use: a PNG chunk with its CRC.

pub(crate) fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    pg_container::png_chunk(kind, data).unwrap()
}
