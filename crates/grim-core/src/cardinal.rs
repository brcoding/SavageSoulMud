use std::fmt;

use serde::{Deserialize, Serialize};

/// The ten compass and vertical directions used for room exits.
///
/// Variant order keeps north, east, south, west, up, down first, then diagonals:
/// the derived `Ord` sorts directions for listings, so keep the variants in
/// this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Cardinal {
    North,
    East,
    South,
    West,
    Up,
    Down,
    Northeast,
    Northwest,
    Southeast,
    Southwest,
}

impl Cardinal {
    /// Parse a direction from a string (case-insensitive, supports abbreviations).
    pub fn parse(input: &str) -> Option<Self> {
        match input.to_lowercase().as_str() {
            "n" | "north" => Some(Self::North),
            "e" | "east" => Some(Self::East),
            "s" | "south" => Some(Self::South),
            "w" | "west" => Some(Self::West),
            "u" | "up" => Some(Self::Up),
            "d" | "down" => Some(Self::Down),
            "ne" | "northeast" => Some(Self::Northeast),
            "nw" | "northwest" => Some(Self::Northwest),
            "se" | "southeast" => Some(Self::Southeast),
            "sw" | "southwest" => Some(Self::Southwest),
            _ => None,
        }
    }

    /// Returns the opposite direction.
    pub fn opposite(self) -> Self {
        match self {
            Self::North => Self::South,
            Self::East => Self::West,
            Self::South => Self::North,
            Self::West => Self::East,
            Self::Up => Self::Down,
            Self::Down => Self::Up,
            Self::Northeast => Self::Southwest,
            Self::Northwest => Self::Southeast,
            Self::Southeast => Self::Northwest,
            Self::Southwest => Self::Northeast,
        }
    }
}

impl fmt::Display for Cardinal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::North => write!(f, "north"),
            Self::East => write!(f, "east"),
            Self::South => write!(f, "south"),
            Self::West => write!(f, "west"),
            Self::Up => write!(f, "up"),
            Self::Down => write!(f, "down"),
            Self::Northeast => write!(f, "northeast"),
            Self::Northwest => write!(f, "northwest"),
            Self::Southeast => write!(f, "southeast"),
            Self::Southwest => write!(f, "southwest"),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::Cardinal;

    #[test]
    fn parse_abbreviation_n() {
        assert_eq!(Cardinal::parse("n"), Some(Cardinal::North));
    }

    #[test]
    fn parse_abbreviation_e() {
        assert_eq!(Cardinal::parse("e"), Some(Cardinal::East));
    }

    #[test]
    fn parse_abbreviation_s() {
        assert_eq!(Cardinal::parse("s"), Some(Cardinal::South));
    }

    #[test]
    fn parse_abbreviation_w() {
        assert_eq!(Cardinal::parse("w"), Some(Cardinal::West));
    }

    #[test]
    fn parse_abbreviation_u() {
        assert_eq!(Cardinal::parse("u"), Some(Cardinal::Up));
    }

    #[test]
    fn parse_abbreviation_d() {
        assert_eq!(Cardinal::parse("d"), Some(Cardinal::Down));
    }

    #[test]
    fn parse_full_north() {
        assert_eq!(Cardinal::parse("north"), Some(Cardinal::North));
    }

    #[test]
    fn parse_full_east() {
        assert_eq!(Cardinal::parse("east"), Some(Cardinal::East));
    }

    #[test]
    fn parse_full_south() {
        assert_eq!(Cardinal::parse("south"), Some(Cardinal::South));
    }

    #[test]
    fn parse_full_west() {
        assert_eq!(Cardinal::parse("west"), Some(Cardinal::West));
    }

    #[test]
    fn parse_full_up() {
        assert_eq!(Cardinal::parse("up"), Some(Cardinal::Up));
    }

    #[test]
    fn parse_full_down() {
        assert_eq!(Cardinal::parse("down"), Some(Cardinal::Down));
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(Cardinal::parse("N"), Some(Cardinal::North));
        assert_eq!(Cardinal::parse("NoRtH"), Some(Cardinal::North));
        assert_eq!(Cardinal::parse("E"), Some(Cardinal::East));
        assert_eq!(Cardinal::parse("S"), Some(Cardinal::South));
        assert_eq!(Cardinal::parse("W"), Some(Cardinal::West));
        assert_eq!(Cardinal::parse("U"), Some(Cardinal::Up));
        assert_eq!(Cardinal::parse("D"), Some(Cardinal::Down));
    }

    #[test]
    fn parse_invalid() {
        assert_eq!(Cardinal::parse("foo"), None);
        assert_eq!(Cardinal::parse("northnorthwest"), None);
        assert_eq!(Cardinal::parse("123"), None);
        assert_eq!(Cardinal::parse(" "), None);
    }

    #[test]
    fn parse_empty() {
        assert_eq!(Cardinal::parse(""), None);
    }

    #[test]
    fn diagonal_roundtrips_and_opposites() {
        for (short, dir) in [
            ("ne", Cardinal::Northeast),
            ("nw", Cardinal::Northwest),
            ("se", Cardinal::Southeast),
            ("sw", Cardinal::Southwest),
        ] {
            assert_eq!(Cardinal::parse(short), Some(dir));
            assert_eq!(Cardinal::parse(&short.to_uppercase()), Some(dir));
            assert_eq!(Cardinal::parse(&dir.to_string()), Some(dir));
            assert_eq!(dir.opposite().opposite(), dir);
        }
        assert_eq!(Cardinal::Northeast.opposite(), Cardinal::Southwest);
        assert_eq!(Cardinal::Northwest.opposite(), Cardinal::Southeast);
    }

    #[test]
    fn opposite_north() {
        assert_eq!(Cardinal::North.opposite(), Cardinal::South);
    }

    #[test]
    fn opposite_east() {
        assert_eq!(Cardinal::East.opposite(), Cardinal::West);
    }

    #[test]
    fn opposite_south() {
        assert_eq!(Cardinal::South.opposite(), Cardinal::North);
    }

    #[test]
    fn opposite_west() {
        assert_eq!(Cardinal::West.opposite(), Cardinal::East);
    }

    #[test]
    fn opposite_up() {
        assert_eq!(Cardinal::Up.opposite(), Cardinal::Down);
    }

    #[test]
    fn opposite_down() {
        assert_eq!(Cardinal::Down.opposite(), Cardinal::Up);
    }

    #[test]
    fn display_north() {
        assert_eq!(format!("{}", Cardinal::North), "north");
    }

    #[test]
    fn display_east() {
        assert_eq!(format!("{}", Cardinal::East), "east");
    }

    #[test]
    fn display_south() {
        assert_eq!(format!("{}", Cardinal::South), "south");
    }

    #[test]
    fn display_west() {
        assert_eq!(format!("{}", Cardinal::West), "west");
    }

    #[test]
    fn display_up() {
        assert_eq!(format!("{}", Cardinal::Up), "up");
    }

    #[test]
    fn display_down() {
        assert_eq!(format!("{}", Cardinal::Down), "down");
    }
    #[test]
    fn sort_follows_display_order() {
        // Variant order is NESWUD; `sort` on `Cardinal` must match it, not
        // alphabetical (down, east, north, …).
        let mut dirs = vec![
            Cardinal::Down,
            Cardinal::West,
            Cardinal::South,
            Cardinal::East,
            Cardinal::North,
            Cardinal::Up,
            Cardinal::Southwest,
            Cardinal::Southeast,
            Cardinal::Northwest,
            Cardinal::Northeast,
        ];
        dirs.sort();
        assert_eq!(
            dirs,
            vec![
                Cardinal::North,
                Cardinal::East,
                Cardinal::South,
                Cardinal::West,
                Cardinal::Up,
                Cardinal::Down,
                Cardinal::Northeast,
                Cardinal::Northwest,
                Cardinal::Southeast,
                Cardinal::Southwest,
            ]
        );
    }
}
