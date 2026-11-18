//! Rectangular patterns can save a geometry reference and an offset expression for each direction.
//! Older designs without those fields keep their spacing. The last release warns about the newer
//! format, opens what it can read and keeps the rest when saving.
