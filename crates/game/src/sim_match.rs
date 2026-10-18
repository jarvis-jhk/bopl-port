//! A running session of the simulation: the [`World`], its level source and
//! the flow requests (next level, back to character select) handled right
//! after each step, so a step plus its level load is one deterministic unit
//! (the shape the rollback adapter will have, `docs/net.md`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Context, Result};
use bopl_import::GameData;
use bopl_sim::World;
use bopl_sim::data::{Content, LevelData};
use bopl_sim::flow::{self, FlowRequest};
use bopl_sim::input::PlayerInput;
use bopl_sim::player::PlayerSetup;
use bopl_sim::world::MatchSettings;

/// Level data converted for the simulation, loaded on first use.
pub struct LevelCache {
    pub data: Rc<GameData>,
    pub content: Arc<Content>,
    /// Bag index → build index (the 124 levels in build order).
    pub bag: Vec<usize>,
    cache: RefCell<HashMap<usize, Arc<LevelData>>>,
}

impl LevelCache {
    pub fn new(data: Rc<GameData>) -> Result<Self> {
        let content = Arc::new(bopl_content::build_content(&data).context("building sim content")?);
        let bag = data.level_indices();
        Ok(Self {
            data,
            content,
            bag,
            cache: RefCell::new(HashMap::new()),
        })
    }

    /// The level with this build index.
    pub fn get(&self, build_index: usize) -> Result<Arc<LevelData>> {
        if let Some(l) = self.cache.borrow().get(&build_index) {
            return Ok(l.clone());
        }
        let lv = self.data.level(build_index)?;
        let l = Arc::new(
            bopl_content::level(&self.content, &lv)
                .with_context(|| format!("converting level {build_index}"))?,
        );
        self.cache.borrow_mut().insert(build_index, l.clone());
        Ok(l)
    }

    pub fn build_index(&self, bag: usize) -> usize {
        self.bag.get(bag).copied().unwrap_or(6)
    }

    pub fn bag_index(&self, build_index: usize) -> usize {
        self.bag.iter().position(|&b| b == build_index).unwrap_or(0)
    }
}

/// One session (character select → … → back). `Clone` is the rollback
/// snapshot online (the level cache is shared).
#[derive(Clone)]
pub struct SimMatch {
    pub world: World,
    pub levels: Rc<LevelCache>,
    /// Build index of the current level.
    pub level: usize,
    /// Bumped whenever a level is loaded (the presentation rebuilds).
    pub level_generation: u32,
    /// The players chose "back to character select".
    pub ended: bool,
}

impl SimMatch {
    /// A new session; the first level is Level1 unless `first_level` (a
    /// build index) is given (sandbox, bot matches).
    pub fn new(
        levels: Rc<LevelCache>,
        setups: &[PlayerSetup],
        settings: MatchSettings,
        first_level: Option<usize>,
    ) -> Result<Self> {
        let mut world = World::new(levels.content.clone(), setups, settings);
        flow::begin_session(&mut world, levels.bag.len());
        let level = first_level.unwrap_or_else(|| levels.build_index(0));
        let lv = levels.get(level)?;
        flow::load_level(&mut world, levels.bag_index(level), lv);
        Ok(Self {
            world,
            levels,
            level,
            level_generation: 1,
            ended: false,
        })
    }

    /// One tick; loads the next level when the flow asks for it.
    pub fn advance(&mut self, inputs: &[PlayerInput]) -> Result<()> {
        if self.ended {
            return Ok(());
        }
        self.world.step(inputs);
        match self.world.flow.request.take() {
            Some(FlowRequest::NextLevel(bag)) => {
                let bi = self.levels.build_index(bag);
                let lv = self.levels.get(bi)?;
                flow::load_level(&mut self.world, bag, lv);
                self.level = bi;
                self.level_generation += 1;
                log::info!("next level: {bi} (bag {bag})");
            }
            Some(FlowRequest::BackToSelect) => self.ended = true,
            None => {}
        }
        Ok(())
    }
}
