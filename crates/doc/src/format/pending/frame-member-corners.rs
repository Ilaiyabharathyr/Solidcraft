//! A frame member saves its `corner` (none, mitre or butt), its `flip` (a butt's other member runs
//! through), `coped` (#17854: every butt joint of the frame cut to the other tube's real shape;
//! saved only when true) and `corners`, the corners finished their own way, each as the lines of
//! its two members, a `corner` and a `flip`. An older design has none of these, so every member
//! keeps its ends square to its line as before. A design from a test build of this change that
//! says `coped_butt` (the frame's or an override's) is read as `butt` with `coped` as it opens
//! (`Document::from_json`, `frame_corners::read_coped`), not by a step: such a design already lists
//! this change as applied. A build before this change opens a design that uses them with a
//! newer-format warning and keeps the fields it cannot read.
