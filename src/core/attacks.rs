//! Attack tables. Leapers (knight, king, pawn) are computed at compile time.
//! Sliders (bishop, rook, queen) use magic bitboards: the magic numbers below
//! were found offline with a seeded search and are verified by the tests.

use std::sync::LazyLock;

use super::{Bitboard, Color, Square};

const fn leaper_table(deltas: &[(i8, i8)]) -> [Bitboard; 64] {
    let mut table = [Bitboard::EMPTY; 64];
    let mut sq = 0;
    while sq < 64 {
        let (file, rank) = ((sq % 8) as i8, (sq / 8) as i8);
        let mut bits = 0u64;
        let mut i = 0;
        while i < deltas.len() {
            let (f, r) = (file + deltas[i].0, rank + deltas[i].1);
            if f >= 0 && f < 8 && r >= 0 && r < 8 {
                bits |= 1 << (r * 8 + f) as u32;
            }
            i += 1;
        }
        table[sq] = Bitboard(bits);
        sq += 1;
    }
    table
}

static KNIGHT: [Bitboard; 64] = leaper_table(&[
    (1, 2),
    (2, 1),
    (2, -1),
    (1, -2),
    (-1, -2),
    (-2, -1),
    (-2, 1),
    (-1, 2),
]);
static KING: [Bitboard; 64] = leaper_table(&[
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
]);
static PAWN: [[Bitboard; 64]; 2] = [
    leaper_table(&[(-1, 1), (1, 1)]),
    leaper_table(&[(-1, -1), (1, -1)]),
];

/// Squares a knight on `sq` attacks.
pub fn knight_attacks(sq: Square) -> Bitboard {
    KNIGHT[sq.index()]
}

/// Squares a king on `sq` attacks.
pub fn king_attacks(sq: Square) -> Bitboard {
    KING[sq.index()]
}

/// Squares a pawn of `color` standing on `sq` attacks.
pub fn pawn_attacks(color: Color, sq: Square) -> Bitboard {
    PAWN[color.index()][sq.index()]
}

/// Squares a bishop on `sq` attacks, given occupancy `occupied`; the ray
/// includes the first blocker.
pub fn bishop_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    let s = &*SLIDERS;
    s.table[s.bishop[sq.index()].index(occupied)]
}

/// Squares a rook on `sq` attacks, given occupancy `occupied`; the ray
/// includes the first blocker.
pub fn rook_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    let s = &*SLIDERS;
    s.table[s.rook[sq.index()].index(occupied)]
}

/// Squares a queen on `sq` attacks, given occupancy `occupied`; the ray
/// includes the first blocker.
pub fn queen_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    bishop_attacks(sq, occupied) | rook_attacks(sq, occupied)
}

/// Squares strictly between `a` and `b` when they share a rank, file or diagonal;
/// empty otherwise.
pub fn between(a: Square, b: Square) -> Bitboard {
    SLIDERS.between[a.index()][b.index()]
}

/// The whole edge-to-edge line through `a` and `b` (both included) when they share
/// a rank, file or diagonal; empty otherwise.
pub fn line(a: Square, b: Square) -> Bitboard {
    SLIDERS.line[a.index()][b.index()]
}

const ROOK_DIRS: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
const BISHOP_DIRS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

/// Reference implementation: walk each ray until the edge or the first blocker.
fn sliding_attacks(sq: Square, occupied: u64, dirs: &[(i8, i8); 4]) -> u64 {
    let mut attacks = 0;
    for &(df, dr) in dirs {
        let (mut f, mut r) = (sq.file() as i8, sq.rank() as i8);
        loop {
            f += df;
            r += dr;
            if !(0..8).contains(&f) || !(0..8).contains(&r) {
                break;
            }
            let bit = 1u64 << (r * 8 + f);
            attacks |= bit;
            if occupied & bit != 0 {
                break;
            }
        }
    }
    attacks
}

/// Squares whose occupancy can change the attack set: each ray minus its edge square.
fn relevant_mask(sq: Square, dirs: &[(i8, i8); 4]) -> u64 {
    let mut mask = 0;
    for &(df, dr) in dirs {
        let (mut f, mut r) = (sq.file() as i8, sq.rank() as i8);
        loop {
            f += df;
            r += dr;
            let (nf, nr) = (f + df, r + dr);
            if !(0..8).contains(&nf) || !(0..8).contains(&nr) {
                break;
            }
            mask |= 1u64 << (r * 8 + f);
        }
    }
    mask
}

struct Magic {
    mask: u64,
    magic: u64,
    shift: u32,
    offset: usize,
}

impl Magic {
    fn index(&self, occupied: Bitboard) -> usize {
        self.offset + ((occupied.0 & self.mask).wrapping_mul(self.magic) >> self.shift) as usize
    }
}

struct Sliders {
    rook: [Magic; 64],
    bishop: [Magic; 64],
    table: Vec<Bitboard>,
    between: Box<[[Bitboard; 64]; 64]>,
    line: Box<[[Bitboard; 64]; 64]>,
}

static SLIDERS: LazyLock<Sliders> = LazyLock::new(Sliders::build);

impl Sliders {
    fn build() -> Sliders {
        let mut table = Vec::new();
        let rook = build_magics(&ROOK_MAGICS, &ROOK_DIRS, &mut table);
        let bishop = build_magics(&BISHOP_MAGICS, &BISHOP_DIRS, &mut table);

        let mut between = Box::new([[Bitboard::EMPTY; 64]; 64]);
        let mut line = Box::new([[Bitboard::EMPTY; 64]; 64]);
        for a in Square::all() {
            for b in Square::all() {
                for dirs in [&ROOK_DIRS, &BISHOP_DIRS] {
                    if sliding_attacks(a, 0, dirs) & b.bb().0 != 0 {
                        between[a.index()][b.index()] = Bitboard(
                            sliding_attacks(a, b.bb().0, dirs) & sliding_attacks(b, a.bb().0, dirs),
                        );
                        line[a.index()][b.index()] = Bitboard(
                            (sliding_attacks(a, 0, dirs) & sliding_attacks(b, 0, dirs))
                                | a.bb().0
                                | b.bb().0,
                        );
                    }
                }
            }
        }
        Sliders {
            rook,
            bishop,
            table,
            between,
            line,
        }
    }
}

fn build_magics(
    magics: &[u64; 64],
    dirs: &[(i8, i8); 4],
    table: &mut Vec<Bitboard>,
) -> [Magic; 64] {
    std::array::from_fn(|i| {
        let sq = Square::from_index_unchecked(i as u8);
        let mask = relevant_mask(sq, dirs);
        let bits = mask.count_ones();
        let magic = Magic {
            mask,
            magic: magics[i],
            shift: 64 - bits,
            offset: table.len(),
        };
        table.resize(table.len() + (1 << bits), Bitboard::EMPTY);
        // Carry-rippler: visit every subset of `mask`.
        let mut subset = 0u64;
        loop {
            table[magic.index(Bitboard(subset))] = Bitboard(sliding_attacks(sq, subset, dirs));
            subset = subset.wrapping_sub(mask) & mask;
            if subset == 0 {
                break;
            }
        }
        magic
    })
}

#[rustfmt::skip]
const ROOK_MAGICS: [u64; 64] = [
    0x1080004008801020,    0x0840092002C03000,    0x1900200010400900,    0x0880100008000480,
    0x4200100420080200,    0x8100020100080400,    0x0200040110886200,    0x0200008040220411,
    0x0404800084400220,    0x0000401000402000,    0x0086001081220440,    0x0408800800100280,
    0x000A001201040820,    0x8848800200840080,    0x4001000100040200,    0x0442000102105084,
    0x9080010020804100,    0x0040404000201009,    0x0000808010002009,    0x2200090021D00100,
    0x0008008008040080,    0x0004004002010040,    0x0011040008015042,    0x00000A0001768104,
    0x0000800080204009,    0x2010004140002001,    0x9800200280100080,    0x1000100080080080,
    0x0442000A00049020,    0x2100040080020080,    0x0800120400900148,    0x0010040A00128541,
    0x2800804000800030,    0x1010002000400041,    0x4000200011004100,    0x0610008410800800,
    0x0400802402800800,    0xC100020080800400,    0x0002000802000401,    0x0182085882000401,
    0x0220204000808000,    0x2860100040024022,    0x0001002004110040,    0x99101042000A0020,
    0x0004080004008080,    0x0010040002008080,    0x2012004881020004,    0x8300842444820011,
    0x0088403882010200,    0x0820400080210100,    0x0110910040A00300,    0x0801100280080480,
    0x0242009008200600,    0x1002000489500200,    0x0040800200010080,    0x0091800041000080,
    0x0000209300488001,    0x04C1002414824001,    0x020020000B001041,    0x7000100004200901,
    0x8002002004100802,    0x30010002084C0007,    0x0888221800813004,    0x4000002840840112,
];
#[rustfmt::skip]
const BISHOP_MAGICS: [u64; 64] = [
    0xA010041108003100,    0x006082020A002900,    0x6810010619200000,    0x08281A0520000408,
    0x0001104001000400,    0x0018901008048400,    0x00040A0210245280,    0x000200210808A402,
    0x9140048410821200,    0x0800091010820041,    0x20504804832202C0,    0x0100091401081000,
    0x8021011140000012,    0x0810020804450400,    0x208B0542109008A2,    0x0080084A08040204,
    0x0040E2A80811244C,    0x2505022008008108,    0x0430220100420040,    0x010A040420220040,
    0x1105000290400000,    0x0093001200822120,    0x4000A62048043004,    0x280120048A015004,
    0x006090002A020814,    0x44042000240800D0,    0x01102800040A4400,    0x1004080080220040,
    0x0001001011004024,    0x0010044000805040,    0x0914041200820100,    0x0004821012821480,
    0x0024040500C05021,    0x0088611002080200,    0x0116080A00040020,    0x4000020080080080,
    0x2450450140840040,    0x0000880201484100,    0x0222020404020092,    0x8081110600002E00,
    0x2842101105000801,    0x1100809008001025,    0x00020202221C0400,    0x0422014022009020,
    0x0210046102100C00,    0xC004008082029102,    0x00AA461801101200,    0x0404080080201108,
    0x020542108C205002,    0x0410544804100100,    0x0040910841100000,    0x0400200042021100,
    0x00004204850400C0,    0x0200100410A42102,    0x1040020801210102,    0x0805040410420000,
    0x2884804130100200,    0x800C262201242000,    0x1058000194108800,    0x0014221054420204,
    0x0104000012A02200,    0x0200881003300100,    0x0140400202840100,    0x0402020801010201,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(s: &str) -> Square {
        s.parse().unwrap()
    }

    fn squares(list: &[&str]) -> Bitboard {
        list.iter().fold(Bitboard::EMPTY, |b, s| b | sq(s).bb())
    }

    #[test]
    fn leaper_tables_respect_edges() {
        assert_eq!(knight_attacks(sq("a1")), squares(&["b3", "c2"]));
        assert_eq!(knight_attacks(sq("h8")), squares(&["g6", "f7"]));
        assert_eq!(knight_attacks(sq("e4")).count(), 8);
        assert_eq!(king_attacks(sq("a1")), squares(&["a2", "b1", "b2"]));
        assert_eq!(king_attacks(sq("e4")).count(), 8);
        assert_eq!(pawn_attacks(Color::White, sq("a2")), squares(&["b3"]));
        assert_eq!(pawn_attacks(Color::White, sq("e4")), squares(&["d5", "f5"]));
        assert_eq!(pawn_attacks(Color::Black, sq("h7")), squares(&["g6"]));
    }

    #[test]
    fn slider_attacks_stop_at_blockers() {
        let occ = squares(&["e6", "c4", "b7"]);
        assert_eq!(
            rook_attacks(sq("e4"), occ),
            squares(&["e5", "e6", "e3", "e2", "e1", "f4", "g4", "h4", "d4", "c4"])
        );
        assert_eq!(
            bishop_attacks(sq("d5"), occ),
            squares(&["e6", "c6", "b7", "c4", "e4", "f3", "g2", "h1"])
        );
    }

    /// Checks every magic against the reference ray walk for every subset of
    /// its relevant mask, plus noise on irrelevant squares.
    #[test]
    fn magics_match_reference() {
        for sq in Square::all() {
            for dirs in [&ROOK_DIRS, &BISHOP_DIRS] {
                let mask = relevant_mask(sq, dirs);
                let mut subset = 0u64;
                loop {
                    let occ = subset | (!mask & 0x8142_2418_1824_4281);
                    let got = if dirs == &ROOK_DIRS {
                        rook_attacks(sq, Bitboard(occ))
                    } else {
                        bishop_attacks(sq, Bitboard(occ))
                    };
                    assert_eq!(got.0, sliding_attacks(sq, occ, dirs), "square {sq}");
                    subset = subset.wrapping_sub(mask) & mask;
                    if subset == 0 {
                        break;
                    }
                }
            }
        }
    }

    #[test]
    fn between_and_line() {
        assert_eq!(between(sq("a1"), sq("a4")), squares(&["a2", "a3"]));
        assert_eq!(between(sq("c3"), sq("f6")), squares(&["d4", "e5"]));
        assert_eq!(between(sq("a1"), sq("b3")), Bitboard::EMPTY);
        assert_eq!(between(sq("e4"), sq("e5")), Bitboard::EMPTY);
        assert_eq!(line(sq("c1"), sq("a1")), Bitboard::RANK_1);
        assert_eq!(line(sq("b2"), sq("d4")).count(), 8);
        assert_eq!(line(sq("a1"), sq("b3")), Bitboard::EMPTY);
    }
}
