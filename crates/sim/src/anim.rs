//! `SpriteAnimator`: the frame-counted animation clock whose end callbacks
//! drive simulation timing (engine.md §10a, player.md §6.7).
//!
//! Callbacks are plain codes ([`AnimCallback`]); [`SpriteAnimator::advance`]
//! returns the callback to fire and the owner dispatches it.

use crate::data::{AnimRef, Content, SpeedFrame, SpriteId};
use crate::fix::Fix;

/// What an animation end callback does (dispatched by the animator's owner).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnimCallback {
    /// Bopl: `Spawn()` after the spawn animation.
    BoplSpawn,
    /// Bopl: stop → idle.
    BoplStopDone,
    /// Bopl: turn → run with the remembered alignment.
    BoplTurnDone(Fix),
    /// Bopl: land animation done (state idle).
    BoplLandDone,
    /// Bopl: bite → chew (victim object index/generation).
    BoplChew,
    BoplSwallow,
    BoplFinishEating,
    /// Ability modules: a module-defined code.
    Ability(u16),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AnimMode {
    #[default]
    Looped,
    /// Sprite chosen by the watched body's vertical speed (`map` = speed
    /// animation of `data`), falling back to the looped animation.
    SpeedY { data: u16, map: u16 },
}

/// Animator state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpriteAnimator {
    pub mode: AnimMode,
    pub current: Option<AnimRef>,
    pub next: Option<AnimRef>,
    pub index: i32,
    pub pos: i32,
    pub callback: Option<AnimCallback>,
    pub callback_on_last_frame: bool,
    pub swapped_this_update: bool,
    pub always_stop_during_time_stop: bool,
    /// Presentation: current sprite and mirroring.
    pub sprite: SpriteId,
    pub flip_x: bool,
}

impl Default for SpriteAnimator {
    fn default() -> SpriteAnimator {
        SpriteAnimator {
            mode: AnimMode::Looped,
            current: None,
            next: None,
            index: 0,
            pos: 0,
            callback: None,
            callback_on_last_frame: false,
            swapped_this_update: false,
            always_stop_during_time_stop: false,
            sprite: SpriteId::NONE,
            flip_x: false,
        }
    }
}

impl SpriteAnimator {
    fn first_frame(&mut self, c: &Content) {
        if let Some(a) = self.current {
            let anim = c.anim(a);
            if let Some(f) = anim.frames.first() {
                self.sprite = f.sprite;
                self.flip_x = f.duration < 0;
            }
        }
    }

    /// `beginAnimation(a)`.
    pub fn begin(&mut self, c: &Content, a: AnimRef) {
        self.mode = AnimMode::Looped;
        self.pos = 0;
        self.index = 0;
        self.current = Some(a);
        self.next = None;
        self.callback = None;
        self.callback_on_last_frame = false;
        self.flip_x = false;
        self.first_frame(c);
    }

    /// `beginAnimThenPlayAnim(a, b[, cb])` (keeps `callBackOnLastFrame`).
    pub fn begin_then_play(
        &mut self,
        c: &Content,
        a: AnimRef,
        b: AnimRef,
        cb: Option<AnimCallback>,
    ) {
        self.mode = AnimMode::Looped;
        self.pos = 0;
        self.index = 0;
        self.current = Some(a);
        self.next = Some(b);
        self.flip_x = false;
        self.callback = cb;
        self.first_frame(c);
    }

    /// `beginAnim(a, cb)`.
    pub fn begin_with_callback(&mut self, c: &Content, a: AnimRef, cb: AnimCallback) {
        self.begin(c, a);
        self.callback = Some(cb);
    }

    /// `beginAnimThenDoAction(a, cb)`.
    pub fn begin_then_do(&mut self, c: &Content, a: AnimRef, cb: AnimCallback) {
        self.begin(c, a);
        self.callback_on_last_frame = true;
        self.callback = Some(cb);
    }

    /// `beginYSpeedBasedAnimation(map, loop)`.
    pub fn begin_speed_y(&mut self, c: &Content, data: u16, map: u16, looped: AnimRef, vy: Fix) {
        self.mode = AnimMode::SpeedY { data, map };
        self.pos = 0;
        self.index = 0;
        self.current = Some(looped);
        self.flip_x = false;
        self.next = None;
        self.callback = None;
        let _ = self.speed_y(c, data, map, vy);
    }

    /// `CurrentSpriteIndex()`.
    pub fn current_index(&self) -> i32 {
        self.index
    }

    /// `getNumberOfPassedSprites()`.
    pub fn passed_sprites(&self) -> i32 {
        self.index + 1
    }

    /// One `UpdateAnimations()`; `vy` is the watched body's vertical speed.
    /// Returns the callback to invoke (already cleared where the original
    /// clears it).
    pub fn advance(&mut self, c: &Content, vy: Fix) -> Option<AnimCallback> {
        match self.mode {
            AnimMode::Looped => self.looped(c),
            AnimMode::SpeedY { data, map } => self.speed_y(c, data, map, vy),
        }
    }

    fn looped(&mut self, c: &Content) -> Option<AnimCallback> {
        let mut cur = self.current?;
        self.pos += 1;
        self.swapped_this_update = false;
        let mut wrapped = false;
        let mut i = self.index;
        while i < c.anim(cur).frames.len() as i32 {
            let frames = &c.anim(cur).frames;
            let d = frames[self.index as usize].duration.abs();
            if self.pos <= d {
                break;
            }
            self.pos -= d;
            self.index += 1;
            if self.index >= frames.len() as i32 {
                wrapped = true;
                self.index = 0;
                if let Some(n) = self.next.take() {
                    cur = n;
                    self.current = Some(n);
                    self.swapped_this_update = true;
                }
            }
            i += 1;
        }
        if !(self.callback_on_last_frame && wrapped)
            && let Some(f) = c.anim(cur).frames.get(self.index as usize)
        {
            self.sprite = f.sprite;
            self.flip_x = f.duration < 0;
        }
        if wrapped && self.callback.is_some() {
            return self.callback.take();
        }
        None
    }

    fn speed_y(&mut self, c: &Content, data: u16, map: u16, vy: Fix) -> Option<AnimCallback> {
        let frames: &[SpeedFrame] =
            &c.anim_data[data as usize].speed_animations[map as usize].frames;
        if frames.is_empty() {
            return None;
        }
        let mut num = 0;
        let mut i = 0;
        while i < frames.len() {
            if vy >= frames[i].from && vy <= frames[i].to {
                num = i;
                break;
            }
            i += 1;
        }
        if vy > frames[0].to {
            num = 0;
        } else if i >= frames.len() {
            return self.looped(c);
        }
        if num >= frames.len() - 1 {
            self.sprite = frames[frames.len() - 1].sprite;
            return self.callback;
        }
        self.sprite = frames[num].sprite;
        None
    }
}
