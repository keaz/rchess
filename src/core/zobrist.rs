//! Zobrist hashing keys, generated at compile time from a fixed seed.

pub(crate) struct Keys {
    pub pieces: [[[u64; 64]; 6]; 2],
    /// XORed in when Black is to move.
    pub side: u64,
    pub castling: [u64; 16],
    pub ep_file: [u64; 8],
}

pub(crate) static KEYS: Keys = build();

const fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

const fn build() -> Keys {
    let mut state = 0x5EED_C0DE_u64;
    let mut keys = Keys {
        pieces: [[[0; 64]; 6]; 2],
        side: 0,
        castling: [0; 16],
        ep_file: [0; 8],
    };
    let mut color = 0;
    while color < 2 {
        let mut kind = 0;
        while kind < 6 {
            let mut sq = 0;
            while sq < 64 {
                keys.pieces[color][kind][sq] = splitmix64(&mut state);
                sq += 1;
            }
            kind += 1;
        }
        color += 1;
    }
    keys.side = splitmix64(&mut state);
    let mut i = 0;
    while i < 16 {
        keys.castling[i] = splitmix64(&mut state);
        i += 1;
    }
    let mut f = 0;
    while f < 8 {
        keys.ep_file[f] = splitmix64(&mut state);
        f += 1;
    }
    keys
}
