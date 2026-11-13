//! A frame member saves its `corner` (none, mitre, butt or coped_butt), its `flip` (a butt's other
//! member runs through) and `corners`, the corners finished their own way, each as the lines of
//! its two members, a `corner` and a `flip`. An older design has none of the three, so every
//! member keeps its ends square to its line as before. A build before this change opens such a
//! design with a newer-format warning and keeps the fields it cannot read.
