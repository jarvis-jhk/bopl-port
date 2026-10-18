//! Global simulation constants (engine.md §1, §7; player.md; physics.md).

use crate::fix::Fix;

/// `GameTime.FixedTimeStep = One / 60` (the tick `dt`).
pub const FIXED_DT: Fix = Fix(71_582_788);
/// `Updater.standardFixedDeltaTime = (Fix)(1f/60f)` (one use, player.md §6.3).
pub const STANDARD_FIXED_DT: Fix = Fix(71_582_792);

pub const CAMERA_X_MIN: Fix = Fix(-417_771_454_464);
pub const CAMERA_X_MAX: Fix = Fix(419_188_801_536);
pub const CAMERA_Y_MAX: Fix = Fix(171_798_691_840);
pub const CAMERA_Y_MIN: Fix = Fix(-111_669_149_696);
pub const BLAST_X_MIN: Fix = Fix(-450_971_566_080);
pub const BLAST_X_MAX: Fix = Fix(450_971_566_080);
pub const BLAST_Y_MAX: Fix = Fix(249_108_103_168);
/// `SceneBounds.waterHeight` (non-space levels, −11.3f).
pub const WATER_HEIGHT: Fix = Fix(-48_533_131_264);
/// `SceneBounds.spaceWaterHeight` (−50).
pub const SPACE_WATER_HEIGHT: Fix = Fix(-214_748_364_800);
/// `Constants.WATER_HEIGHT` (−11.22f; beams, pickups).
pub const CONST_WATER_HEIGHT: Fix = Fix(-48_189_534_208);

/// `Constants.DEADZONE` (0.4f).
pub const DEADZONE: Fix = Fix(1_717_986_944);
/// `Constants.GROUNDALIGNMENTDEADZONE` (0.2f).
pub const GROUND_ALIGNMENT_DEADZONE: Fix = Fix(858_993_472);
/// `Constants.MIN_TURNSPEED` (0.02f).
pub const MIN_TURNSPEED: Fix = Fix(85_899_344);
/// Player id of the environment (water, blast zone, kill zones).
pub const ENVIRONMENT_ID: i32 = 1000;
/// `Constants.MaxClones`.
pub const MAX_CLONES: i32 = 16;

/// `PhysTools.posCorrectionStr` (0.3f).
pub const POS_CORRECTION_STR: Fix = Fix(1_288_490_240);
/// `PhysTools.slop` (0.01f).
pub const SLOP: Fix = Fix(42_949_672);

/// `(Fix)0.01f`.
pub const F_0_01: Fix = Fix(42_949_672);
/// `(Fix)0.05f`.
pub const F_0_05: Fix = Fix(214_748_368);
/// `(Fix)0.2f`.
pub const F_0_2: Fix = Fix(858_993_472);
/// `(Fix)0.25f`.
pub const F_0_25: Fix = Fix(1_073_741_824);
/// `(Fix)0.5` / `0.5f`.
pub const F_0_5: Fix = Fix(2_147_483_648);
/// `(Fix)0.001f`.
pub const F_0_001: Fix = Fix(4_294_967);
/// `(Fix)0.95` (double).
pub const F_0_95: Fix = Fix(4_080_218_931);
/// `(Fix)0.3f`.
pub const F_0_3: Fix = Fix(1_288_490_240);
/// `(Fix)0.3` (double).
pub const F_0_3D: Fix = Fix(1_288_490_188);
/// `(Fix)0.01` (double).
pub const F_0_01D: Fix = Fix(42_949_672);
/// `(Fix)0.1` (double).
pub const F_0_1D: Fix = Fix(429_496_729);
/// `(Fix)0.1f`.
pub const F_0_1: Fix = Fix(429_496_736);
/// `(Fix)0.9` (double).
pub const F_0_9D: Fix = Fix(3_865_470_566);
/// `(Fix)0.07` (double).
pub const F_0_07D: Fix = Fix(300_647_710);
/// `(Fix)1.47f`.
pub const F_1_47: Fix = Fix(6_313_602_048);
/// `(Fix)(−1.5f)`.
pub const F_NEG_1_5: Fix = Fix(-6_442_450_944);
/// `(Fix)0.87f` (ice slipperiness).
pub const ICE_SLIP: Fix = Fix(3_736_621_568);
/// Normal ground slipperiness `0.5f`.
pub const NORMAL_SLIP: Fix = Fix(2_147_483_648);
