//! Skate 3's trick catalogue: every scorable trick by its identifier (the executable's `EScorableID`
//! enum: its numeric class and score type, from SK8 `skate-core/src/scoring/catalog.rs`, GPL-3.0, which
//! names it TU3 table820862A8), and each trick's points and completion delay (Skate 3's scoring
//! records, class `Hash_6918469984A8C596`, SK8 `skate-data/src/scoring.rs`).
//!
//! The built-in points are Skate 3's stock ones for the tricks flatground can do (read off the disc's
//! scoring records with `fg_import values`); a rules pack's `rules/scoring` entry carries the disc's
//! record for every trick it has.
//!
//! `rules/scoring` entry: `u16` count, then per trick `u16` id, `i32` points, `f32` completion delay (s).
//! All big-endian.

use alloc::vec::Vec;

use crate::gesture::{Flick, Trick};

/// The entry's name in the rules pack.
pub const ENTRY: &str = "rules/scoring";

/// How many scorable tricks Skate 3's enum has.
pub const COUNT: usize = 332;

/// Class of a trick (the enum's metadata): 0 ground, 1 grind, 2 grab, 3 flip, 4 jump, 5 air metric,
/// 6 handplant.
pub const CLASS_GRAB: u8 = 2;
pub const CLASS_METRIC: u8 = 5;
pub const CLASS_HANDPLANT: u8 = 6;
/// Score types the collectors treat apart: a manual, a powerslide.
pub const TYPE_MANUAL: u8 = 8;
pub const TYPE_SLIDE: u8 = 9;

/// The enum: identifier, class, score type; the index is the id.
pub const IDENTIFIERS: [(&str, u8, u8); COUNT] = [
    ("fspowerslide", 0, 9),
    ("bspowerslide", 0, 9),
    ("nosemanual", 0, 8),
    ("tailmanual", 0, 8),
    ("fsrevert", 0, 11),
    ("bsrevert", 0, 11),
    ("bs_5_o", 1, 5),
    ("bs_50_50", 1, 5),
    ("bs_blunt", 1, 5),
    ("bs_board", 1, 5),
    ("bs_crook", 1, 5),
    ("bs_feeble", 1, 5),
    ("bs_lip", 1, 5),
    ("bs_noseblunt", 1, 5),
    ("bs_nosegrind", 1, 5),
    ("bs_noseslide", 1, 5),
    ("bs_overcrook", 1, 5),
    ("bs_overwilly", 1, 5),
    ("bs_salad", 1, 5),
    ("bs_smith", 1, 5),
    ("bs_tailslide", 1, 5),
    ("bs_willy", 1, 5),
    ("fs_5_o", 1, 5),
    ("fs_50_50", 1, 5),
    ("fs_blunt", 1, 5),
    ("fs_board", 1, 5),
    ("fs_crook", 1, 5),
    ("fs_feeble", 1, 5),
    ("fs_lip", 1, 5),
    ("fs_noseblunt", 1, 5),
    ("fs_nosegrind", 1, 5),
    ("fs_noseslide", 1, 5),
    ("fs_overcrook", 1, 5),
    ("fs_overwilly", 1, 5),
    ("fs_salad", 1, 5),
    ("fs_smith", 1, 5),
    ("fs_tailslide", 1, 5),
    ("fs_willy", 1, 5),
    ("grind5_o", 1, 5),
    ("grind5050", 1, 5),
    ("grindcrooks", 1, 5),
    ("grindfeeble", 1, 5),
    ("grindnose", 1, 5),
    ("grindovercrooks", 1, 5),
    ("grindoverwilly", 1, 5),
    ("grindsalad", 1, 5),
    ("grindsmith", 1, 5),
    ("grindwilly", 1, 5),
    ("slideblunt", 1, 5),
    ("slideboard", 1, 5),
    ("slidelip", 1, 5),
    ("slidenblunt", 1, 5),
    ("slidenose", 1, 5),
    ("slidetail", 1, 5),
    ("bsgrab", 2, 4),
    ("bsgrab_down", 2, 4),
    ("bsgrab_left", 2, 4),
    ("bsgrab_right", 2, 4),
    ("bsgrab_up", 2, 4),
    ("christ_air", 2, 4),
    ("coffin", 2, 4),
    ("dblgrab", 2, 4),
    ("fsgrab", 2, 4),
    ("fsgrab_down", 2, 4),
    ("fsgrab_left", 2, 4),
    ("fsgrab_right", 2, 4),
    ("fsgrab_up", 2, 4),
    ("gnd_bsgrab", 2, 4),
    ("gnd_dblgrab", 2, 4),
    ("gnd_fsgrab", 2, 4),
    ("mutegrab", 2, 4),
    ("mutegrab_down", 2, 4),
    ("mutegrab_left", 2, 4),
    ("mutegrab_up", 2, 4),
    ("nosegrab", 2, 4),
    ("nosegrab_left", 2, 4),
    ("nosegrab_right", 2, 4),
    ("rocketair", 2, 4),
    ("stalegrab", 2, 4),
    ("stalegrab_down", 2, 4),
    ("stalegrab_left", 2, 4),
    ("stalegrab_up", 2, 4),
    ("tailgrab", 2, 4),
    ("tailgrab_left", 2, 4),
    ("tailgrab_right", 2, 4),
    ("360flip", 3, 2),
    ("360hardflip", 3, 2),
    ("360inwardheelflip", 3, 2),
    ("360popshuvit", 3, 2),
    ("fs360popshuvit", 3, 2),
    ("fspopshuvit", 3, 2),
    ("hardflip", 3, 2),
    ("heelflip", 3, 2),
    ("heelflip2", 3, 2),
    ("heelflip3", 3, 2),
    ("inwardheelflip", 3, 2),
    ("kickflip", 3, 2),
    ("kickflip2", 3, 2),
    ("kickflip3", 3, 2),
    ("laserflip", 3, 2),
    ("latebackfootheelflip", 3, 2),
    ("latebackfootkickflip", 3, 2),
    ("latebsshuv", 3, 2),
    ("latefsshuv", 3, 2),
    ("lateheelflip", 3, 2),
    ("latekickflip", 3, 2),
    ("n_360flip", 3, 2),
    ("n_360hardflip", 3, 2),
    ("n_360inwardheelflip", 3, 2),
    ("n_360popshuvit", 3, 2),
    ("n_fs360popshuvit", 3, 2),
    ("n_fspopshuvit", 3, 2),
    ("n_hardflip", 3, 2),
    ("n_heelflip", 3, 2),
    ("n_heelflip2", 3, 2),
    ("n_heelflip3", 3, 2),
    ("n_inwardheelflip", 3, 2),
    ("n_kickflip", 3, 2),
    ("n_kickflip2", 3, 2),
    ("n_kickflip3", 3, 2),
    ("n_laserflip", 3, 2),
    ("n_popshuvit", 3, 2),
    ("n_varialheelflip", 3, 2),
    ("n_varialkickflip", 3, 2),
    ("popshuvit", 3, 2),
    ("varialheelflip", 3, 2),
    ("varialkickflip", 3, 2),
    ("nollie", 4, 1),
    ("ollie", 4, 1),
    ("air_horizontal_distance", 5, 0),
    ("air_height_to_peak", 5, 0),
    ("air_total_height_gain", 5, 0),
    ("air_player_spin", 5, 0),
    ("air_player_flip", 5, 0),
    ("heelflip4", 3, 2),
    ("kickflip4", 3, 2),
    ("n_heelflip4", 3, 2),
    ("n_kickflip4", 3, 2),
    ("nofoot_air", 2, 4),
    ("fs_handplant", 6, 6),
    ("fs_handplant_up", 6, 6),
    ("fs_handplant_right", 6, 6),
    ("fs_handplant_left", 6, 6),
    ("fs_handplant_down", 6, 6),
    ("fs_handplant_onefoot_front", 6, 6),
    ("fs_handplant_onefoot_back", 6, 6),
    ("fs_handplant_nofoot", 6, 6),
    ("fs_handplant_to_fakie", 6, 6),
    ("fs_handplant_to_revert", 6, 6),
    ("bs_handplant", 6, 6),
    ("bs_handplant_up", 6, 6),
    ("bs_handplant_right", 6, 6),
    ("bs_handplant_left", 6, 6),
    ("bs_handplant_down", 6, 6),
    ("bs_handplant_onefoot_front", 6, 6),
    ("bs_handplant_onefoot_back", 6, 6),
    ("bs_handplant_nofoot", 6, 6),
    ("bs_handplant_to_fakie", 6, 6),
    ("bs_handplant_to_revert", 6, 6),
    ("bs_handplant_mount", 6, 6),
    ("bs_handplant_dismount", 6, 6),
    ("bs_handplant_to_flat", 6, 6),
    ("fs_onefoot_right", 2, 4),
    ("fs_backfoot", 2, 4),
    ("bs_onefoot_front", 2, 4),
    ("bs_onefoot_behind", 2, 4),
    ("nosegrab_onefoot_front", 2, 4),
    ("nosegrab_onefoot_behind", 2, 4),
    ("tailgrab_onefoot_front", 2, 4),
    ("tailgrab_onefoot_behind", 2, 4),
    ("ollie_onefoot_front", 4, 1),
    ("ollie_onefoot_back", 4, 1),
    ("tailgrab_airwalk", 2, 4),
    ("fsgrab_fingerflip", 4, 3),
    ("bsgrab_fingerflip", 4, 3),
    ("nosegrab_fingerflip", 4, 3),
    ("tailgrab_fingerflip", 4, 3),
    ("mutegrab_fingerflip", 4, 3),
    ("stalegrab_fingerflip", 4, 3),
    ("fsgrabvarial_fingerflip", 4, 3),
    ("bsgrabvarial_fingerflip", 4, 3),
    ("fs_rock_n_roll", 1, 5),
    ("bs_rock_n_roll", 1, 5),
    ("fakie_rock", 1, 5),
    ("dropin", 1, 5),
    ("dropin_fs_revert", 1, 5),
    ("dropin_bs_revert", 1, 5),
    ("crailgrab", 2, 4),
    ("crailgrab_left", 2, 4),
    ("crailgrab_right", 2, 4),
    ("seatbeltgrab", 2, 4),
    ("seatbeltgrab_left", 2, 4),
    ("seatbeltgrab_right", 2, 4),
    ("fs_50_50_fsgrab", 1, 5),
    ("fs_50_50_bsgrab", 1, 5),
    ("fs_nosegrind_tailgrab", 1, 5),
    ("fs_nosegrind_bsgrab", 1, 5),
    ("fs_crook_tailgrab", 1, 5),
    ("fs_crook_bsgrab", 1, 5),
    ("bs_crook_tailgrab", 1, 5),
    ("bs_crook_bsgrab", 1, 5),
    ("fs_noseslide_tailgrab", 1, 5),
    ("bs_noseslide_tailgrab", 1, 5),
    ("fs_noseblunt_tailgrab", 1, 5),
    ("bs_noseblunt_tailgrab", 1, 5),
    ("fs_5_o_fsgrab", 1, 5),
    ("fs_5_o_nosegrab", 1, 5),
    ("fs_salad_fsgrab", 1, 5),
    ("fs_salad_nosegrab", 1, 5),
    ("bs_salad_fsgrab", 1, 5),
    ("bs_salad_nosegrab", 1, 5),
    ("fs_tailslide_nosegrab", 1, 5),
    ("bs_tailslide_nosegrab", 1, 5),
    ("fs_blunt_tailgrab", 1, 5),
    ("bs_blunt_tailgrab", 1, 5),
    ("fsboneless", 4, 12),
    ("bsboneless", 4, 12),
    ("fsfastplant", 4, 12),
    ("bsfastplant", 4, 12),
    ("bsgrab_fs_footplant", 4, 13),
    ("bsgrab_bs_footplant", 4, 13),
    ("fsgrab_fs_footplant", 4, 13),
    ("fsgrab_bs_footplant", 4, 13),
    ("nosegrab_fs_footplant", 4, 13),
    ("nosegrab_bs_footplant", 4, 13),
    ("tailgrab_fs_footplant", 4, 13),
    ("tailgrab_bs_footplant", 4, 13),
    ("ollie_fs_footplant", 4, 13),
    ("ollie_bs_footplant", 4, 13),
    ("nocomply", 4, 7),
    ("no_comply_fs180", 4, 7),
    ("no_comply_bs180", 4, 7),
    ("no_comply_fs360", 4, 7),
    ("no_comply_bs360", 4, 7),
    ("hippyjump", 4, 10),
    ("hippyjump_fs180", 4, 10),
    ("hippyjump_bs180", 4, 10),
    ("air_metric", 5, 0),
    ("fsgrabtostalegrab_fingerflip", 4, 3),
    ("bsgrabtomutegrab_fingerflip", 4, 3),
    ("stalegrabtofsgrab_fingerflip", 4, 3),
    ("mutegrabtobsgrab_fingerflip", 4, 3),
    ("bs_50_50_fsgrab", 1, 5),
    ("bs_50_50_bsgrab", 1, 5),
    ("bs_nosegrind_tailgrab", 1, 5),
    ("bs_nosegrind_bsgrab", 1, 5),
    ("bs_5_o_fsgrab", 1, 5),
    ("bs_5_o_nosegrab", 1, 5),
    ("fs_overcrook_tailgrab", 1, 5),
    ("fs_overcrook_bsgrab", 1, 5),
    ("bs_overcrook_tailgrab", 1, 5),
    ("bs_overcrook_bsgrab", 1, 5),
    ("superman", 2, 4),
    ("generic_metric", 5, 0),
    ("nosegrab_airwalk", 2, 4),
    ("seatbelttonose_fingerflip", 4, 3),
    ("crailtotail_fingerflip", 4, 3),
    ("fs_board_double", 1, 5),
    ("fs_board_tail", 1, 5),
    ("fs_board_nose", 1, 5),
    ("bs_board_double", 1, 5),
    ("bs_board_tail", 1, 5),
    ("bs_board_nose", 1, 5),
    ("fs_lip_double", 1, 5),
    ("fs_lip_tail", 1, 5),
    ("fs_lip_nose", 1, 5),
    ("bs_lip_double", 1, 5),
    ("bs_lip_tail", 1, 5),
    ("bs_lip_nose", 1, 5),
    ("bs_darkslide", 1, 5),
    ("fs_darkslide", 1, 5),
    ("bs_darkslide_double", 1, 5),
    ("bs_darkslide_tail", 1, 5),
    ("bs_darkslide_nose", 1, 5),
    ("fs_darkslide_double", 1, 5),
    ("fs_darkslide_tail", 1, 5),
    ("fs_darkslide_nose", 1, 5),
    ("heelflip_underflip", 3, 2),
    ("heelflip2_underflip", 3, 2),
    ("heelflip3_underflip", 3, 2),
    ("heelflip4_underflip", 3, 2),
    ("kickflip_underflip", 3, 2),
    ("kickflip2_underflip", 3, 2),
    ("kickflip3_underflip", 3, 2),
    ("kickflip4_underflip", 3, 2),
    ("n_heelflip_underflip", 3, 2),
    ("n_heelflip2_underflip", 3, 2),
    ("n_heelflip3_underflip", 3, 2),
    ("n_heelflip4_underflip", 3, 2),
    ("n_kickflip_underflip", 3, 2),
    ("n_kickflip2_underflip", 3, 2),
    ("n_kickflip3_underflip", 3, 2),
    ("n_kickflip4_underflip", 3, 2),
    ("popshuvit_underflip", 3, 2),
    ("fspopshuvit_underflip", 3, 2),
    ("n_popshuvit_underflip", 3, 2),
    ("n_fspopshuvit_underflip", 3, 2),
    ("heelflip_darkcatch", 3, 2),
    ("heelflip2_darkcatch", 3, 2),
    ("heelflip3_darkcatch", 3, 2),
    ("heelflip4_darkcatch", 3, 2),
    ("kickflip_darkcatch", 3, 2),
    ("kickflip2_darkcatch", 3, 2),
    ("kickflip3_darkcatch", 3, 2),
    ("kickflip4_darkcatch", 3, 2),
    ("n_heelflip_darkcatch", 3, 2),
    ("n_heelflip2_darkcatch", 3, 2),
    ("n_heelflip3_darkcatch", 3, 2),
    ("n_heelflip4_darkcatch", 3, 2),
    ("n_kickflip_darkcatch", 3, 2),
    ("n_kickflip2_darkcatch", 3, 2),
    ("n_kickflip3_darkcatch", 3, 2),
    ("n_kickflip4_darkcatch", 3, 2),
    ("360flip_darkcatch", 3, 2),
    ("360hardflip_darkcatch", 3, 2),
    ("360inwardheelflip_darkcatch", 3, 2),
    ("laserflip_darkcatch", 3, 2),
    ("n_360flip_darkcatch", 3, 2),
    ("n_360hardflip_darkcatch", 3, 2),
    ("n_360inwardheelflip_darkcatch", 3, 2),
    ("n_laserflip_darkcatch", 3, 2),
    ("darktolightbsshuv", 3, 2),
    ("darktolightfsshuv", 3, 2),
    ("darktolightheelflip", 3, 2),
    ("darktolightkickflip", 3, 2),
    ("darkslideout_bsstraight", 3, 2),
    ("darkslideout_bsleft", 3, 2),
    ("darkslideout_bsright", 3, 2),
    ("darkslideout_fsstraight", 3, 2),
    ("darkslideout_fsleft", 3, 2),
    ("darkslideout_fsright", 3, 2),
    ("lateflip_darkcatch", 3, 2),
];

/// Skate 3's points and completion delay (s) for the tricks flatground does.
const BUILT_IN: &[(&str, i32, f32)] = &[
    ("fspowerslide", 40, 0.4),
    ("bspowerslide", 40, 0.4),
    ("nosemanual", 40, 0.2),
    ("tailmanual", 40, 0.2),
    ("fsrevert", 10, 0.0),
    ("bsrevert", 10, 0.0),
    ("bs_5_o", 40, 0.2),
    ("bs_50_50", 20, 0.2),
    ("bs_blunt", 150, 0.2),
    ("bs_board", 50, 0.2),
    ("bs_crook", 60, 0.2),
    ("bs_feeble", 150, 0.2),
    ("bs_lip", 50, 0.2),
    ("bs_noseblunt", 150, 0.2),
    ("bs_nosegrind", 40, 0.2),
    ("bs_noseslide", 80, 0.2),
    ("bs_overcrook", 60, 0.2),
    ("bs_overwilly", 80, 0.2),
    ("bs_salad", 60, 0.2),
    ("bs_smith", 150, 0.2),
    ("bs_tailslide", 80, 0.2),
    ("bs_willy", 20, 0.2),
    ("fs_5_o", 40, 0.2),
    ("fs_50_50", 20, 0.2),
    ("fs_blunt", 150, 0.2),
    ("fs_board", 50, 0.2),
    ("fs_crook", 60, 0.2),
    ("fs_feeble", 150, 0.2),
    ("fs_lip", 50, 0.2),
    ("fs_noseblunt", 150, 0.2),
    ("fs_nosegrind", 40, 0.2),
    ("fs_noseslide", 80, 0.2),
    ("fs_overcrook", 60, 0.2),
    ("fs_overwilly", 80, 0.2),
    ("fs_salad", 60, 0.2),
    ("fs_smith", 150, 0.2),
    ("fs_tailslide", 80, 0.2),
    ("fs_willy", 20, 0.2),
    ("grind5_o", 20, 0.2),
    ("grind5050", 20, 0.2),
    ("grindcrooks", 40, 0.2),
    ("grindfeeble", 120, 0.2),
    ("grindnose", 20, 0.2),
    ("grindovercrooks", 40, 0.2),
    ("grindoverwilly", 120, 0.2),
    ("grindsalad", 40, 0.2),
    ("grindsmith", 120, 0.2),
    ("grindwilly", 60, 0.2),
    ("slideblunt", 100, 0.2),
    ("slideboard", 50, 0.2),
    ("slidelip", 50, 0.2),
    ("slidenblunt", 100, 0.2),
    ("slidenose", 75, 0.2),
    ("slidetail", 75, 0.2),
    ("bsgrab", 20, 0.2),
    ("bsgrab_down", 60, 0.2),
    ("bsgrab_left", 60, 0.2),
    ("bsgrab_right", 60, 0.2),
    ("bsgrab_up", 60, 0.2),
    ("christ_air", 100, 0.4),
    ("coffin", 20, 0.5),
    ("dblgrab", 20, 0.2),
    ("fsgrab", 20, 0.2),
    ("fsgrab_down", 60, 0.2),
    ("fsgrab_left", 60, 0.2),
    ("fsgrab_right", 60, 0.2),
    ("fsgrab_up", 60, 0.2),
    ("gnd_bsgrab", 0, 0.2),
    ("gnd_dblgrab", 0, 0.2),
    ("gnd_fsgrab", 0, 0.2),
    ("mutegrab", 120, 0.2),
    ("mutegrab_down", 140, 0.2),
    ("mutegrab_left", 140, 0.2),
    ("mutegrab_up", 140, 0.2),
    ("nosegrab", 80, 0.2),
    ("nosegrab_left", 100, 0.2),
    ("nosegrab_right", 100, 0.2),
    ("rocketair", 300, 0.2),
    ("stalegrab", 120, 0.2),
    ("stalegrab_down", 140, 0.2),
    ("stalegrab_left", 140, 0.2),
    ("stalegrab_up", 140, 0.2),
    ("tailgrab", 80, 0.2),
    ("tailgrab_left", 100, 0.2),
    ("tailgrab_right", 100, 0.2),
    ("360flip", 250, 0.0),
    ("360hardflip", 300, 0.0),
    ("360inwardheelflip", 300, 0.0),
    ("360popshuvit", 150, 0.0),
    ("fs360popshuvit", 150, 0.0),
    ("fspopshuvit", 100, 0.0),
    ("hardflip", 225, 0.0),
    ("heelflip", 100, 0.0),
    ("heelflip2", 150, 0.0),
    ("heelflip3", 200, 0.0),
    ("inwardheelflip", 225, 0.0),
    ("kickflip", 100, 0.0),
    ("kickflip2", 150, 0.0),
    ("kickflip3", 200, 0.0),
    ("laserflip", 250, 0.0),
    ("latebackfootheelflip", 150, 0.0),
    ("latebackfootkickflip", 150, 0.0),
    ("latebsshuv", 150, 0.0),
    ("latefsshuv", 150, 0.0),
    ("lateheelflip", 150, 0.0),
    ("latekickflip", 150, 0.0),
    ("n_360flip", 275, 0.0),
    ("n_360hardflip", 300, 0.0),
    ("n_360inwardheelflip", 300, 0.0),
    ("n_360popshuvit", 150, 0.0),
    ("n_fs360popshuvit", 150, 0.0),
    ("n_fspopshuvit", 100, 0.0),
    ("n_hardflip", 250, 0.0),
    ("n_heelflip", 100, 0.0),
    ("n_heelflip2", 150, 0.0),
    ("n_heelflip3", 200, 0.0),
    ("n_inwardheelflip", 250, 0.0),
    ("n_kickflip", 100, 0.0),
    ("n_kickflip2", 150, 0.0),
    ("n_kickflip3", 200, 0.0),
    ("n_laserflip", 275, 0.0),
    ("n_popshuvit", 100, 0.0),
    ("n_varialheelflip", 200, 0.0),
    ("n_varialkickflip", 200, 0.0),
    ("popshuvit", 100, 0.0),
    ("varialheelflip", 200, 0.0),
    ("varialkickflip", 200, 0.0),
    ("nollie", 10, 0.0),
    ("ollie", 10, 0.0),
    ("heelflip4", 250, 0.0),
    ("kickflip4", 250, 0.0),
    ("n_heelflip4", 250, 0.0),
    ("n_kickflip4", 250, 0.0),
    ("fs_50_50_fsgrab", 40, 0.2),
    ("fs_50_50_bsgrab", 40, 0.2),
    ("fs_nosegrind_tailgrab", 60, 0.2),
    ("fs_nosegrind_bsgrab", 40, 0.2),
    ("fs_crook_tailgrab", 60, 0.2),
    ("fs_crook_bsgrab", 40, 0.2),
    ("bs_crook_tailgrab", 60, 0.2),
    ("bs_crook_bsgrab", 40, 0.2),
    ("fs_noseslide_tailgrab", 80, 0.2),
    ("bs_noseslide_tailgrab", 80, 0.2),
    ("fs_noseblunt_tailgrab", 150, 0.2),
    ("bs_noseblunt_tailgrab", 150, 0.2),
    ("fs_5_o_fsgrab", 40, 0.2),
    ("fs_5_o_nosegrab", 60, 0.2),
    ("fs_salad_fsgrab", 40, 0.2),
    ("fs_salad_nosegrab", 60, 0.2),
    ("bs_salad_fsgrab", 40, 0.2),
    ("bs_salad_nosegrab", 60, 0.2),
    ("fs_tailslide_nosegrab", 80, 0.2),
    ("bs_tailslide_nosegrab", 80, 0.2),
    ("fs_blunt_tailgrab", 150, 0.2),
    ("bs_blunt_tailgrab", 150, 0.2),
    ("fsboneless", 100, 0.0),
    ("bsboneless", 100, 0.0),
    ("fsfastplant", 100, 0.0),
    ("bsfastplant", 100, 0.0),
    ("bsgrab_fs_footplant", 100, 0.4),
    ("bsgrab_bs_footplant", 100, 0.4),
    ("fsgrab_fs_footplant", 100, 0.4),
    ("fsgrab_bs_footplant", 100, 0.4),
    ("nocomply", 200, 0.0),
    ("hippyjump", 60, 0.0),
    ("bs_50_50_fsgrab", 40, 0.2),
    ("bs_50_50_bsgrab", 40, 0.2),
    ("bs_nosegrind_tailgrab", 60, 0.2),
    ("bs_nosegrind_bsgrab", 40, 0.2),
    ("bs_5_o_fsgrab", 40, 0.2),
    ("bs_5_o_nosegrab", 60, 0.2),
    ("fs_overcrook_tailgrab", 60, 0.2),
    ("fs_overcrook_bsgrab", 40, 0.2),
    ("bs_overcrook_tailgrab", 60, 0.2),
    ("bs_overcrook_bsgrab", 40, 0.2),
    ("bs_darkslide", 50, 0.2),
    ("fs_darkslide", 50, 0.2),
    ("fs_handplant", 100, 0.2),
    ("fs_handplant_up", 150, 0.2),
    ("fs_handplant_right", 150, 0.2),
    ("fs_handplant_left", 150, 0.2),
    ("fs_handplant_down", 150, 0.2),
    ("fs_handplant_onefoot_front", 150, 0.0),
    ("fs_handplant_onefoot_back", 150, 0.0),
    ("fs_handplant_nofoot", 150, 0.0),
    ("bs_handplant", 100, 0.2),
    ("bs_handplant_up", 150, 0.2),
    ("bs_handplant_right", 150, 0.2),
    ("bs_handplant_left", 150, 0.2),
    ("bs_handplant_down", 150, 0.2),
    ("bs_handplant_onefoot_front", 150, 0.0),
    ("bs_handplant_onefoot_back", 150, 0.0),
    ("bs_handplant_nofoot", 150, 0.0),
];

/// A trick's id by its identifier (any case: the state graphs' names are upper case).
pub fn id(name: &str) -> Option<u16> {
    IDENTIFIERS
        .iter()
        .position(|(n, _, _)| n.eq_ignore_ascii_case(name))
        .map(|i| i as u16)
}

/// A trick's id by its identifier written in parts (`n_`, `kickflip`, `2`), without building it.
pub fn id_of(parts: &[&str]) -> Option<u16> {
    IDENTIFIERS
        .iter()
        .position(|(n, _, _)| {
            let mut rest = n.as_bytes();
            for p in parts {
                let p = p.as_bytes();
                if rest.len() < p.len() || !rest[..p.len()].eq_ignore_ascii_case(p) {
                    return false;
                }
                rest = &rest[p.len()..];
            }
            rest.is_empty()
        })
        .map(|i| i as u16)
}

/// The identifier of a trick id.
pub fn name(id: u16) -> &'static str {
    IDENTIFIERS.get(id as usize).map_or("", |r| r.0)
}

/// A trick's class and score type.
pub fn kind(id: u16) -> (u8, u8) {
    IDENTIFIERS.get(id as usize).map_or((0, 0), |r| (r.1, r.2))
}

/// A flip's id: `n_` for a nollie, the flip count of a double to quadruple kick or heel flip, a late
/// flip's own record, `_darkcatch` caught dark when Skate 3 has one.
pub fn flip_id(f: Flick, flips: u8, late: bool, dark: bool) -> Option<u16> {
    let base = match f.trick {
        Trick::Ollie => return id(if f.nollie { "nollie" } else { "ollie" }),
        Trick::Kickflip => "kickflip",
        Trick::Heelflip => "heelflip",
        Trick::PopShuvit => "popshuvit",
        Trick::FsPopShuvit => "fspopshuvit",
        Trick::Bs360Shuvit => "360popshuvit",
        Trick::Fs360Shuvit => "fs360popshuvit",
        Trick::VarialKickflip => "varialkickflip",
        Trick::VarialHeelflip => "varialheelflip",
        Trick::Hardflip => "hardflip",
        Trick::InwardHeelflip => "inwardheelflip",
        Trick::TreFlip => "360flip",
        Trick::Laserflip => "laserflip",
        Trick::Hardflip360 => "360hardflip",
        Trick::InwardHeelflip360 => "360inwardheelflip",
    };
    if late && !f.nollie {
        let late = match f.trick {
            Trick::Kickflip => Some("latekickflip"),
            Trick::Heelflip => Some("lateheelflip"),
            Trick::PopShuvit => Some("latebsshuv"),
            Trick::FsPopShuvit => Some("latefsshuv"),
            _ => None,
        };
        if let Some(l) = late.and_then(id) {
            return Some(l);
        }
    }
    let n = if f.nollie { "n_" } else { "" };
    const COUNTS: [&str; 5] = ["", "", "2", "3", "4"];
    let count = if matches!(f.trick, Trick::Kickflip | Trick::Heelflip) {
        COUNTS[flips.clamp(1, 4) as usize]
    } else {
        ""
    };
    if dark && let Some(d) = id_of(&[n, base, count, "_darkcatch"]) {
        return Some(d);
    }
    id_of(&[n, base, count]).or_else(|| id_of(&[n, base]))
}

/// A grab's id: the state graphs' scored grab names are Skate 3's identifiers; flatground's own grab
/// names map to them (an indy is Skate 3's frontside grab, a melon its backside grab).
pub fn grab_id(name: &str) -> Option<u16> {
    id(name).or_else(|| {
        id(match name {
            "INDY" => "fsgrab",
            "MELON" => "bsgrab",
            "TAILGRAB" => "tailgrab",
            "NOSEGRAB" => "nosegrab",
            "STALEFISH" => "stalegrab",
            "MUTE" => "mutegrab",
            "DOUBLE GRAB" => "dblgrab",
            _ => return None,
        })
    })
}

/// Points and completion delays by trick id: Skate 3's for the tricks flatground does, or the disc's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Catalog {
    points: [i32; COUNT],
    delay: [f32; COUNT],
    has: [bool; COUNT],
    /// Came from a rules pack.
    pub imported: bool,
}

impl Default for Catalog {
    fn default() -> Self {
        Catalog::built_in()
    }
}

impl Catalog {
    pub fn built_in() -> Catalog {
        let mut c = Catalog {
            points: [0; COUNT],
            delay: [0.0; COUNT],
            has: [false; COUNT],
            imported: false,
        };
        for &(n, p, d) in BUILT_IN {
            if let Some(i) = id(n) {
                c.set(i, p, d);
            }
        }
        c
    }

    fn set(&mut self, id: u16, points: i32, delay: f32) {
        let i = id as usize;
        if i < COUNT {
            self.points[i] = points;
            self.delay[i] = if delay.is_finite() {
                delay.max(0.0)
            } else {
                0.0
            };
            self.has[i] = true;
        }
    }

    /// The trick's points and completion delay, if it has a record.
    pub fn get(&self, id: u16) -> Option<(i32, f32)> {
        let i = id as usize;
        (i < COUNT && self.has[i]).then(|| (self.points[i], self.delay[i]))
    }

    /// How many tricks have a record.
    pub fn len(&self) -> usize {
        self.has.iter().filter(|h| **h).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Takes over a `rules/scoring` entry: its tricks replace the built-in catalogue. `None` (and nothing
    /// changed) if it is malformed or empty.
    pub fn parse(data: &[u8]) -> Option<Catalog> {
        let be = |at: usize| -> Option<[u8; 4]> { data.get(at..at + 4)?.try_into().ok() };
        let n = u16::from_be_bytes(data.get(0..2)?.try_into().ok()?) as usize;
        if n == 0 || data.len() != 2 + n * 10 {
            return None;
        }
        let mut c = Catalog {
            points: [0; COUNT],
            delay: [0.0; COUNT],
            has: [false; COUNT],
            imported: true,
        };
        for k in 0..n {
            let at = 2 + k * 10;
            let id = u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?);
            if id as usize >= COUNT {
                return None;
            }
            let points = i32::from_be_bytes(be(at + 2)?);
            let delay = f32::from_be_bytes(be(at + 6)?);
            c.set(id, points, delay);
        }
        Some(c)
    }
}

/// Encodes tricks (id, points, completion delay) as a `rules/scoring` entry.
pub fn encode(tricks: &[(u16, i32, f32)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + tricks.len() * 10);
    out.extend_from_slice(&(tricks.len() as u16).to_be_bytes());
    for (id, points, delay) in tricks {
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&points.to_be_bytes());
        out.extend_from_slice(&delay.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tricks_are_found_by_their_identifier_in_any_case_or_in_parts() {
        assert_eq!(id("kickflip"), Some(96));
        assert_eq!(id("FS_50_50"), Some(23));
        assert_eq!(id("ollie"), Some(128));
        assert_eq!(id("no such trick"), None);
        assert_eq!(id_of(&["n_", "kickflip", "2"]), id("n_kickflip2"));
        assert_eq!(id_of(&["FS_5_O", "_", "NOSEGRAB"]), Some(206));
        assert_eq!(id_of(&["kick", "flip", "9"]), None);
        assert_eq!(name(96), "kickflip");
        assert_eq!(kind(96), (3, 2));
        assert_eq!(kind(id("fsgrab").unwrap()), (CLASS_GRAB, 4));
        assert_eq!(kind(id("nosemanual").unwrap()).1, TYPE_MANUAL);
    }

    #[test]
    fn flatgrounds_tricks_have_skate_3s_ids() {
        let k = Flick::of(Trick::Kickflip);
        assert_eq!(flip_id(k, 1, false, false), id("kickflip"));
        assert_eq!(flip_id(k, 3, false, false), id("kickflip3"));
        assert_eq!(flip_id(k, 2, false, true), id("kickflip2_darkcatch"));
        assert_eq!(flip_id(k, 1, true, false), id("latekickflip"));
        let n = Flick {
            trick: Trick::TreFlip,
            nollie: true,
        };
        assert_eq!(flip_id(n, 1, false, false), id("n_360flip"));
        assert_eq!(flip_id(Flick::NOLLIE, 1, false, false), id("nollie"));
        assert_eq!(
            flip_id(Flick::of(Trick::Fs360Shuvit), 1, false, true),
            id("fs360popshuvit")
        );
        assert_eq!(
            flip_id(Flick::of(Trick::Laserflip), 1, true, false),
            id("laserflip")
        );
        assert_eq!(grab_id("INDY"), id("fsgrab"));
        assert_eq!(grab_id("STALEFISH"), id("stalegrab"));
        assert_eq!(grab_id("MuteGrab_Down"), id("mutegrab_down"));
        assert_eq!(grab_id("NOTHING"), None);
        // Every trick flatground can do has a record in the built-in catalogue: flicks, grinds (the
        // graphs' names, `BF_` ones as their side's), foot jumps and air foot plants.
        let c = Catalog::built_in();
        for g in crate::skater::GRAPH_NAMES {
            let n = g.strip_prefix("BF_").unwrap_or(g);
            assert!(id(n).and_then(|i| c.get(i)).is_some(), "{g}");
        }
        use crate::skater::plant::{PlantKind, footplant_name};
        for k in [
            PlantKind::FsBoneless,
            PlantKind::BsFastplant,
            PlantKind::NoComply,
        ] {
            assert!(id(k.trick()).and_then(|i| c.get(i)).is_some(), "{k:?}");
        }
        for (a, b) in [(true, true), (false, false)] {
            assert!(id(footplant_name(a, b)).and_then(|i| c.get(i)).is_some());
        }
        for g in [
            "INDY",
            "MELON",
            "TAILGRAB",
            "NOSEGRAB",
            "STALEFISH",
            "MUTE",
            "DOUBLE GRAB",
        ] {
            assert!(grab_id(g).and_then(|i| c.get(i)).is_some(), "{g}");
        }
        for t in crate::gesture::TRICKS {
            for nollie in [false, true] {
                let f = Flick { trick: t, nollie };
                assert!(
                    c.get(flip_id(f, 1, false, false).unwrap()).is_some(),
                    "{f:?}"
                );
            }
        }
    }

    #[test]
    fn the_built_in_catalogue_has_skate_3s_points_and_a_rules_entry_replaces_it() {
        let c = Catalog::built_in();
        assert!(!c.imported);
        assert_eq!(c.get(96), Some((100, 0.0)), "kickflip");
        assert_eq!(c.get(id("ollie").unwrap()), Some((10, 0.0)));
        assert_eq!(c.get(id("fs_50_50").unwrap()), Some((20, 0.2)));
        assert_eq!(
            c.get(id("air_player_spin").unwrap()),
            None,
            "metrics have no record"
        );
        // Every built-in identifier is in the enum.
        assert_eq!(c.len(), BUILT_IN.len());
        let data = encode(&[(96, 120, 0.0), (23, 25, 0.25)]);
        let i = Catalog::parse(&data).unwrap();
        assert!(i.imported);
        assert_eq!(i.get(96), Some((120, 0.0)));
        assert_eq!(i.get(23), Some((25, 0.25)));
        assert_eq!(i.get(128), None, "only what the disc has");
        assert_eq!(i.len(), 2);
        assert_eq!(Catalog::parse(&data[..5]), None);
        assert_eq!(Catalog::parse(&encode(&[])), None);
        assert_eq!(Catalog::parse(&encode(&[(400, 1, 0.0)])), None);
    }
}
