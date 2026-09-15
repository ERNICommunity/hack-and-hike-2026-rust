//! SHA-256 (FIPS 180-4), used to match an ELF file to the firmware image
//! built from it. The server has no crate dependencies, so it is written out.

use std::io::{self, Read};

const INITIAL_STATE: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const ROUND_CONSTANTS: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// The SHA-256 digest of everything `reader` yields.
pub fn digest_reader(mut reader: impl Read) -> io::Result<[u8; 32]> {
    let mut state = INITIAL_STATE;
    let mut block = [0_u8; 64];
    let mut filled = 0;
    let mut total_bytes: u64 = 0;

    loop {
        let read = reader.read(&mut block[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
        total_bytes += read as u64;
        if filled == block.len() {
            compress(&mut state, &block);
            filled = 0;
        }
    }

    // Padding: a one bit, zeros, then the message length in bits.
    block[filled] = 0x80;
    block[filled + 1..].fill(0);
    if filled >= 56 {
        compress(&mut state, &block);
        block.fill(0);
    }
    block[56..].copy_from_slice(&(total_bytes * 8).to_be_bytes());
    compress(&mut state, &block);

    let mut digest = [0_u8; 32];
    for (chunk, word) in digest.chunks_exact_mut(4).zip(state) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    Ok(digest)
}

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut schedule = [0_u32; 64];
    for (word, bytes) in schedule.iter_mut().zip(block.chunks_exact(4)) {
        *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }
    for index in 16..64 {
        let s15 = schedule[index - 15];
        let s2 = schedule[index - 2];
        schedule[index] = schedule[index - 16]
            .wrapping_add(s15.rotate_right(7) ^ s15.rotate_right(18) ^ (s15 >> 3))
            .wrapping_add(schedule[index - 7])
            .wrapping_add(s2.rotate_right(17) ^ s2.rotate_right(19) ^ (s2 >> 10));
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for (word, constant) in schedule.iter().zip(ROUND_CONSTANTS) {
        let t1 = h
            .wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25))
            .wrapping_add((e & f) ^ (!e & g))
            .wrapping_add(constant)
            .wrapping_add(*word);
        let t2 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
            .wrapping_add((a & b) ^ (a & c) ^ (b & c));
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }

    for (word, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *word = word.wrapping_add(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn matches_the_standard_test_vectors() {
        assert_eq!(
            hex(&digest_reader(&b""[..]).unwrap()),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&digest_reader(&b"abc"[..]).unwrap()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&digest_reader(&b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"[..]).unwrap()),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn handles_every_padding_boundary() {
        // 55, 56 and 64 bytes straddle the one-block / two-block padding cases;
        // a reader that returns one byte at a time exercises partial blocks.
        struct OneByte<'a>(&'a [u8]);
        impl Read for OneByte<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let Some((first, rest)) = self.0.split_first() else {
                    return Ok(0);
                };
                buffer[0] = *first;
                self.0 = rest;
                Ok(1)
            }
        }
        for length in [55, 56, 63, 64, 65, 200] {
            let data = vec![b'a'; length];
            assert_eq!(
                digest_reader(&data[..]).unwrap(),
                digest_reader(OneByte(&data)).unwrap(),
                "length {length}"
            );
        }
        assert_eq!(
            hex(&digest_reader(&[b'a'; 1_000_000][..]).unwrap()),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }
}
