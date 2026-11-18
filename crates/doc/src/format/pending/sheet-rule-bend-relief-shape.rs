//! A sheet metal rule saves `bend_relief_shape` (`obround` or `tear`) when its bend relief is
//! not the rectangular slot, and an Edge Flange feature saves `bend_relief_shape`
//! (`rectangular`, `obround` or `tear`) when it asks for a bend relief other than its rule's.
//! Designs without the fields read them as `rectangular` and the rule's, the only bend relief
//! they could cut, and build as before. The last release warns about the newer design format,
//! opens what it understands (its rules and flanges cut rectangular bend reliefs) and keeps the
//! unfamiliar fields when saving.
