//! Playing an animation tree from the [`MotionDb`](super::MotionDb): clocks, parameters, attribute
//! queries and the list of poses to blend.
//!
//! A port of SK8-ENGINE/skate-3-rust-engine (GPL-3.0) crates/skate-core/src/animation: clip_clock.rs,
//! phase_blend.rs, playback_tree.rs, playback_tree/blend_space.rs, playback_tree/selection_space.rs,
//! playback_attributes.rs, and the selector resolution of crates/skate-game/src/graph_host/
//! motion_animation/tree_builder.rs. SK8 matches the original's fused multiply-adds bit for bit; this port
//! uses plain arithmetic (results agree to rounding).
//!
//! Use: [`Playback::build`] a tree by name, [`Playback::set_parameters`] whenever the game's parameters
//! change, [`Playback::advance`] by the frame time, then [`Playback::evaluate`] for the poses to blend
//! ([`PoseCommand`]; [`leaf_weights`] flattens them to clip weights). Mirroring is done on the blended
//! pose of the whole tree (`anim::mirror_pose`), as SK8's bind-pose wrapper does.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::{
    ClipInfo, KIND_BONE, KIND_CURVE, KIND_SCALAR, KIND_VECTOR, MotionDb, NameId, Node,
    SelectionParam, SimplexDef, TreeDef,
};

/// A clip's clock (SK8 ClipClock). Times in seconds of playback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    pub frames: f32,
    pub fps: f32,
    pub base_speed: f32,
    pub speed: f32,
    pub length: f32,
    pub time: f32,
    pub previous: f32,
    /// Wraps since the last [`Clock::commit`].
    pub loops: u32,
    pub looping: bool,
    pub phase_controlled: bool,
}

/// What an advance did (SK8 AdvanceResult).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Advance {
    /// Went past the end (looping clips wrapped).
    pub crossed_end: bool,
    pub overshoot: f32,
    pub remaining_before_wrap: f32,
}

impl Default for Advance {
    fn default() -> Self {
        Advance {
            crossed_end: false,
            overshoot: -1.0,
            remaining_before_wrap: -1.0,
        }
    }
}

impl Clock {
    pub fn new(c: &ClipInfo) -> Clock {
        Clock {
            frames: c.frames,
            fps: c.fps,
            base_speed: c.base_speed,
            speed: 1.0,
            length: c.length(),
            time: 0.0,
            previous: 0.0,
            loops: 0,
            looping: c.looping(),
            phase_controlled: c.phase_controlled(),
        }
    }
    pub fn set_time(&mut self, time: f32) {
        self.time = time;
        self.previous = time;
        self.loops = 0;
    }
    pub fn set_speed(&mut self, speed: f32) {
        let new_rate = self.base_speed * speed;
        let old_rate = self.speed * self.base_speed;
        self.time = self.time * old_rate / new_rate;
        self.previous = self.previous * old_rate / new_rate;
        self.length = (self.frames - 1.0) / (self.fps * self.base_speed * speed);
        self.speed = speed;
    }
    pub fn sample_time(&self) -> f32 {
        if self.length - self.time >= 0.0 {
            self.time
        } else {
            self.length
        }
    }
    pub fn advance(&mut self, dt: f32, phase: f32, result: &mut Advance) {
        if self.phase_controlled {
            self.previous = self.time;
            self.time = self.length * phase.clamp(0.0, 1.0);
            self.loops = u32::from(self.time < self.previous);
            return;
        }
        self.time += dt;
        result.remaining_before_wrap = self.length - self.time;
        if self.time <= self.length {
            result.crossed_end = false;
        } else if self.looping && self.length > 0.0 && self.time.is_finite() {
            while self.time > self.length {
                result.crossed_end = true;
                result.overshoot = self.time - self.length;
                self.time -= self.length;
                self.loops = self.loops.wrapping_add(1);
            }
        } else {
            result.crossed_end = true;
            result.overshoot = self.time - self.length;
        }
    }
    /// After a pose evaluation: the step starts again here.
    pub fn commit(&mut self) {
        self.previous = self.time;
        self.loops = 0;
    }
}

/// A parameter the game sets on trees (SK8 SettableAttribute).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Param {
    pub name: NameId,
    pub value: f32,
    /// Phase blends only: `value` is 0..1 across the children's range instead of a raw value.
    pub normalized: bool,
}

/// An attribute travelling through the tree (SK8 AnimationAttribute). `lanes` hold raw words: lane 0 the
/// scalar (or vector x), 1..3 the vector's other lanes, 0..4 a bone name and 5 its scalar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attr {
    pub name: NameId,
    pub kind: u8,
    pub lanes: [u32; 6],
    pub begin: f32,
    pub end: f32,
    pub status: u8,
}

impl Attr {
    /// An untimed scalar (SK8 MotionGraphAttribute::to_animation): the starting value of a query.
    pub fn scalar(name: NameId, value: f32) -> Attr {
        let mut lanes = [0; 6];
        lanes[0] = value.to_bits();
        Attr {
            name,
            kind: KIND_SCALAR,
            lanes,
            begin: -1.0,
            end: -1.0,
            status: 6,
        }
    }
    pub fn value(&self) -> f32 {
        f32::from_bits(self.lanes[if self.kind == KIND_BONE { 5 } else { 0 }])
    }
    fn numeric(&self) -> &'static [usize] {
        match self.kind {
            KIND_SCALAR | KIND_CURVE => &[0],
            KIND_VECTOR => &[0, 1, 2, 3],
            KIND_BONE => &[5],
            _ => &[],
        }
    }
    fn blend(&mut self, other: &Attr, w: f32) {
        if self.status & 2 != 0 {
            self.begin = -1.0;
            self.end = -1.0;
        } else {
            self.begin = other.begin * w + self.begin * (1.0 - w);
            self.end = other.end * w + self.end * (1.0 - w);
        }
        for &i in self.numeric() {
            let (a, b) = (
                f32::from_bits(self.lanes[i]),
                f32::from_bits(other.lanes[i]),
            );
            self.lanes[i] = (b * w + a * (1.0 - w)).to_bits();
        }
    }
    fn scale(&mut self, w: f32) {
        if self.status & 2 != 0 {
            self.begin = -1.0;
            self.end = -1.0;
        } else {
            self.begin *= w;
            self.end *= w;
        }
        for &i in self.numeric() {
            self.lanes[i] = (f32::from_bits(self.lanes[i]) * w).to_bits();
        }
    }
    fn add_weighted(&mut self, other: &Attr, w: f32) {
        if self.status & 2 == 0 {
            self.begin += other.begin * w;
            self.end += other.end * w;
        }
        for &i in self.numeric() {
            let (a, b) = (
                f32::from_bits(self.lanes[i]),
                f32::from_bits(other.lanes[i]),
            );
            self.lanes[i] = (b * w + a).to_bits();
        }
    }
}

/// One step of building the pose, in order (a small stack machine; SK8 PoseCommand).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PoseCommand {
    /// Push the clip's pose at `time` (seconds of the clip's own timeline: frame = time * fps). The step
    /// started at `previous` with `loops` wraps in between (for events and root motion).
    Clip {
        clip: u16,
        previous: f32,
        time: f32,
        loops: u32,
    },
    /// Pop two poses, push `a * (1 - weight) + b * weight` (b was pushed last).
    Blend { weight: f32 },
    /// Pop `count` poses, push their sum weighted by `weights` (first pushed first).
    WeightedBlend { weights: [f32; 5], count: u8 },
}

/// Flattens commands into (clip, time, weight) per playing clip; weights sum to 1.
pub fn leaf_weights(commands: &[PoseCommand]) -> Vec<(u16, f32, f32)> {
    let mut stack: Vec<Vec<(u16, f32, f32)>> = Vec::new();
    for c in commands {
        match *c {
            PoseCommand::Clip { clip, time, .. } => stack.push(vec![(clip, time, 1.0)]),
            PoseCommand::Blend { weight } => {
                let (Some(b), Some(mut a)) = (stack.pop(), stack.pop()) else {
                    return Vec::new();
                };
                a.iter_mut().for_each(|l| l.2 *= 1.0 - weight);
                a.extend(b.into_iter().map(|(c, t, w)| (c, t, w * weight)));
                stack.push(a);
            }
            PoseCommand::WeightedBlend { weights, count } => {
                let n = count as usize;
                if stack.len() < n {
                    return Vec::new();
                }
                let parts = stack.split_off(stack.len() - n);
                let mut out = Vec::new();
                for (part, w) in parts.into_iter().zip(weights) {
                    out.extend(part.into_iter().map(|(c, t, x)| (c, t, x * w)));
                }
                stack.push(out);
            }
        }
    }
    stack.pop().unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildError {
    /// A name the library doesn't have (or a clip/tree that wasn't converted).
    Missing(NameId),
    /// A tree refers back to itself.
    Cycle,
    /// A tree whose data can't be played (no children, bad simplex).
    Invalid,
}

/// A playing tree.
#[derive(Clone, Debug)]
pub enum Playback {
    Clip { clip: u16, clock: Clock },
    PhaseBlend(Box<PhaseBlend>),
    BlendSpace(Box<BlendSpace>),
    SelectionSpace(Box<SelectionSpace>),
}

impl Playback {
    /// Builds the tree (or clip) `name`. `construction` holds (key, value) name pairs that choose
    /// selectors' children; a missing key takes the selector's default.
    pub fn build(
        db: &MotionDb,
        name: &str,
        construction: &[(NameId, NameId)],
    ) -> Result<Playback, BuildError> {
        let node = db
            .find(name)
            .ok_or(BuildError::Missing(db.name_id(name).unwrap_or(u16::MAX)))?;
        Self::build_node(db, node, construction, &mut Vec::new())
    }

    pub fn build_node(
        db: &MotionDb,
        node: Node,
        construction: &[(NameId, NameId)],
        parents: &mut Vec<u16>,
    ) -> Result<Playback, BuildError> {
        let index = match node {
            Node::Missing(n) => return Err(BuildError::Missing(n)),
            Node::Clip(c) => {
                return Ok(Playback::Clip {
                    clip: c,
                    clock: Clock::new(db.clip(c)),
                });
            }
            Node::Tree(t) => t,
        };
        if parents.contains(&index) {
            return Err(BuildError::Cycle);
        }
        parents.push(index);
        let mut child = |n: Node| Self::build_node(db, n, construction, parents);
        let built = match &db.tree(index).def {
            TreeDef::Selector {
                parameter,
                default,
                options,
            } => {
                let want = construction
                    .iter()
                    .find(|(k, _)| k == parameter)
                    .map(|(_, v)| *v);
                let pick = options
                    .iter()
                    .find(|(value, _)| Some(*value) == want)
                    .map_or(*default, |(_, n)| *n);
                child(pick)?
            }
            TreeDef::PhaseBlend {
                parameter,
                children,
            } => {
                let built = children
                    .iter()
                    .map(|&n| child(n))
                    .collect::<Result<Vec<_>, _>>()?;
                Playback::PhaseBlend(Box::new(
                    PhaseBlend::new(*parameter, built).ok_or(BuildError::Invalid)?,
                ))
            }
            TreeDef::BlendSpace {
                parameters,
                children,
                simplexes,
            } => {
                let built = children
                    .iter()
                    .map(|&n| child(n))
                    .collect::<Result<Vec<_>, _>>()?;
                Playback::BlendSpace(Box::new(
                    BlendSpace::new(parameters.clone(), built, simplexes.clone())
                        .ok_or(BuildError::Invalid)?,
                ))
            }
            TreeDef::SelectionSpace {
                parameters,
                candidates,
            } => {
                let built = candidates
                    .iter()
                    .map(|(n, values)| Ok((*n, values.clone(), child(*n)?)))
                    .collect::<Result<Vec<_>, BuildError>>()?;
                Playback::SelectionSpace(Box::new(SelectionSpace::new(parameters.clone(), built)))
            }
        };
        parents.pop();
        Ok(built)
    }

    pub fn length(&self) -> f32 {
        match self {
            Playback::Clip { clock, .. } => clock.length,
            Playback::PhaseBlend(t) => t.length,
            Playback::BlendSpace(t) => t.length(),
            Playback::SelectionSpace(t) => t.current().map_or(0.0, Playback::length),
        }
    }

    pub fn time(&self) -> f32 {
        match self {
            Playback::Clip { clock, .. } => clock.sample_time(),
            Playback::PhaseBlend(t) => t.time,
            Playback::BlendSpace(t) => t.time,
            Playback::SelectionSpace(t) => t.current().map_or(0.0, Playback::time),
        }
    }

    pub fn set_time(&mut self, time: f32) {
        match self {
            Playback::Clip { clock, .. } => clock.set_time(time),
            Playback::PhaseBlend(t) => t.set_time(time),
            Playback::BlendSpace(t) => t.set_time(time),
            Playback::SelectionSpace(t) => {
                if let Some(c) = t.current_mut() {
                    c.set_time(time)
                }
            }
        }
    }

    pub fn set_speed(&mut self, speed: f32) {
        match self {
            Playback::Clip { clock, .. } => clock.set_speed(speed),
            Playback::PhaseBlend(t) => t.set_speed(speed),
            Playback::BlendSpace(t) => t.children.iter_mut().for_each(|c| c.set_speed(speed)),
            Playback::SelectionSpace(t) => {
                t.speed = speed;
                if let Some(c) = t.current_mut() {
                    c.set_speed(speed)
                }
            }
        }
    }

    /// Moves time on by `dt` seconds (`phase`: 0..1 for phase-controlled clips).
    pub fn advance(&mut self, dt: f32, phase: f32, result: &mut Advance) {
        match self {
            Playback::Clip { clock, .. } => clock.advance(dt, phase, result),
            Playback::PhaseBlend(t) => t.advance(dt, phase, result),
            Playback::BlendSpace(t) => t.advance(dt, phase, result),
            Playback::SelectionSpace(t) => {
                if let Some(c) = t.current_mut() {
                    c.advance(dt, phase, result)
                }
            }
        }
    }

    /// Hands the game's parameters to every node; returns whether anything changed.
    pub fn set_parameters(&mut self, db: &MotionDb, params: &[Param]) -> bool {
        match self {
            Playback::Clip { .. } => false,
            Playback::PhaseBlend(t) => t.set_parameters(db, params),
            Playback::BlendSpace(t) => t.set_parameters(params),
            Playback::SelectionSpace(t) => t.set_parameters(db, params),
        }
    }

    /// The attribute `name` of the playing clips, blended like their poses. `mask`: 15 active now or
    /// crossed this step, 31 any. `out` is only written when found (start it with [`Attr::scalar`]).
    pub fn query_attribute(&self, db: &MotionDb, name: NameId, mask: u32, out: &mut Attr) -> bool {
        match self {
            Playback::Clip { clip, clock } => {
                match db.attribute_step(
                    *clip,
                    name,
                    clock.previous,
                    clock.time,
                    clock.loops,
                    clock.speed,
                    mask,
                ) {
                    Some(v) => {
                        let mut lanes = [0u32; 6];
                        lanes[0] = v.value.to_bits();
                        if v.kind == KIND_VECTOR {
                            lanes[..4].copy_from_slice(&v.vector.map(f32::to_bits));
                        }
                        if let Some(b) = v.bone {
                            lanes[..5].copy_from_slice(&b);
                            lanes[5] = v.value.to_bits();
                        }
                        *out = Attr {
                            name,
                            kind: v.kind,
                            lanes,
                            begin: v.begin,
                            end: v.end,
                            status: v.status,
                        };
                        true
                    }
                    None => false,
                }
            }
            Playback::PhaseBlend(t) => t.query_attribute(db, name, mask, out),
            Playback::BlendSpace(t) => t.query_attribute(db, name, mask, out),
            Playback::SelectionSpace(t) => t
                .current()
                .is_some_and(|c| c.query_attribute(db, name, mask, out)),
        }
    }

    /// Appends the poses to blend for this frame to `out`. `cull`: weights within this of 0 or 1 skip
    /// the other side (SK8 uses a small threshold such as 0.01). `update_history`: start the next step's
    /// event window here (commit the clocks). Returns whether a pose was produced.
    pub fn evaluate(
        &mut self,
        cull: f32,
        enabled: bool,
        update_history: bool,
        out: &mut Vec<PoseCommand>,
    ) -> bool {
        match self {
            Playback::Clip { clip, clock } => {
                let rate = clock.base_speed * clock.speed;
                let limit = (clock.frames - 1.0) / clock.fps;
                let bound = |t: f32| t.max(0.0).min(limit);
                let mut previous = bound(clock.previous * rate);
                let time = bound(clock.sample_time() * rate);
                if clock.loops == 0 && previous > time {
                    previous = time;
                }
                if enabled {
                    out.push(PoseCommand::Clip {
                        clip: *clip,
                        previous,
                        time,
                        loops: clock.loops,
                    });
                }
                if update_history {
                    clock.commit();
                }
                enabled
            }
            Playback::PhaseBlend(t) => t.evaluate(cull, enabled, update_history, out),
            Playback::BlendSpace(t) => {
                if !enabled {
                    return false;
                }
                let s = &t.simplexes[t.current];
                for &i in &s.children {
                    if !t.children[i as usize].evaluate(cull, true, update_history, out) {
                        return false;
                    }
                }
                let mut weights = [0.0; 5];
                weights[..t.weights.len()].copy_from_slice(&t.weights);
                out.push(PoseCommand::WeightedBlend {
                    weights,
                    count: t.weights.len() as u8,
                });
                true
            }
            Playback::SelectionSpace(t) => {
                enabled
                    && t.current_mut()
                        .is_some_and(|c| c.evaluate(cull, true, update_history, out))
            }
        }
    }
}

/// Two neighbouring children, sorted by their own value of the parameter, blended with clocks in phase.
#[derive(Clone, Debug)]
pub struct PhaseBlend {
    pub parameter: NameId,
    pub children: Vec<Playback>,
    order: Vec<usize>,
    values: Vec<f32>,
    pub left: usize,
    pub right: usize,
    pub weight: f32,
    value: f32,
    normalized: bool,
    cull: f32,
    pub length: f32,
    pub time: f32,
}

impl PhaseBlend {
    fn new(parameter: NameId, children: Vec<Playback>) -> Option<Self> {
        if children.len() < 2 {
            return None;
        }
        let n = children.len();
        Some(PhaseBlend {
            parameter,
            length: children[0].length(),
            children,
            order: (0..n).collect(),
            values: vec![0.0; n],
            left: 0,
            right: 1,
            weight: 0.0,
            value: f32::MAX,
            normalized: true,
            cull: 0.0,
            time: 0.0,
        })
    }
    fn set_time(&mut self, time: f32) {
        let phase = time / self.length;
        let (l, r) = (self.left, self.right);
        let lt = self.children[l].length() * phase;
        let rt = self.children[r].length() * phase;
        self.children[l].set_time(lt);
        self.children[r].set_time(rt);
        self.time = time;
    }
    fn phase(&self) -> f32 {
        self.time / if self.length <= 0.0 { 1.0 } else { self.length }
    }
    fn update_length(&mut self, phase: f32) {
        self.length = self.children[self.right].length() * self.weight
            + self.children[self.left].length() * (1.0 - self.weight);
        self.set_time(self.length * phase);
    }
    fn set_speed(&mut self, speed: f32) {
        let phase = self.phase();
        for c in &mut self.children {
            c.set_speed(speed);
        }
        self.update_length(phase);
    }
    fn advance(&mut self, dt: f32, phase: f32, result: &mut Advance) {
        let ll = self.children[self.left].length();
        let rl = self.children[self.right].length();
        let fraction = dt / self.length;
        self.children[self.left].advance(fraction * ll, phase, result);
        self.children[self.right].advance(fraction * rl, phase, &mut Advance::default());
        let inv = 1.0 / ll;
        self.time = self.children[self.left].time() * inv * self.length;
        result.remaining_before_wrap = result.remaining_before_wrap * inv * self.length;
        result.overshoot *= inv * self.length;
    }
    fn set_parameters(&mut self, db: &MotionDb, params: &[Param]) -> bool {
        let mut changed = false;
        for c in &mut self.children {
            changed |= c.set_parameters(db, params);
        }
        if let Some(p) = params.iter().find(|p| p.name == self.parameter)
            && (p.normalized != self.normalized || (p.value - self.value).abs() >= 1e-4)
        {
            self.value = p.value;
            self.normalized = p.normalized;
            changed = true;
        }
        if !changed {
            return false;
        }
        // Each child's own value of the parameter (an untimed attribute of its clips).
        for i in 0..self.children.len() {
            let mut a = Attr::scalar(self.parameter, 0.0);
            self.children[i].query_attribute(db, self.parameter, 15, &mut a);
            self.values[i] = f32::from_bits(a.lanes[0]);
        }
        let values = &self.values;
        // Stable bubble sort, as the original.
        for end in (1..self.order.len()).rev() {
            for i in 0..end {
                if values[self.order[i + 1]] < values[self.order[i]] {
                    self.order.swap(i, i + 1);
                }
            }
        }
        let min = values[self.order[0]];
        let max = values[*self.order.last().unwrap()];
        let target = if self.normalized {
            (max - min) * self.value + min
        } else {
            self.value
        };
        // First child whose value is not below the target.
        let low = self
            .order
            .iter()
            .position(|&i| values[i] >= target)
            .unwrap_or(self.order.len());
        let upper = low.clamp(1, self.order.len() - 1);
        let (left, right) = (self.order[upper - 1], self.order[upper]);
        let (lv, uv) = (values[left], values[right]);
        let delta = uv - lv;
        let clamped = target.max(lv).min(uv);
        let w = (clamped - lv) / if delta == 0.0 { 1.0 } else { delta };
        self.weight = w.clamp(0.0, 1.0);
        let phase = self.phase();
        self.left = left;
        self.right = right;
        self.update_length(phase);
        true
    }
    fn query_attribute(&self, db: &MotionDb, name: NameId, mask: u32, out: &mut Attr) -> bool {
        if self.weight < self.cull {
            return self.children[self.left].query_attribute(db, name, mask, out);
        }
        if self.weight > 1.0 - self.cull {
            return self.children[self.right].query_attribute(db, name, mask, out);
        }
        let left = self.children[self.left].query_attribute(db, name, mask, out);
        let mut right = Attr::scalar(name, 0.0);
        let found = self.children[self.right].query_attribute(db, name, 31, &mut right);
        if left && found {
            out.blend(&right, self.weight);
            true
        } else {
            false
        }
    }
    fn evaluate(
        &mut self,
        cull: f32,
        enabled: bool,
        update_history: bool,
        out: &mut Vec<PoseCommand>,
    ) -> bool {
        self.cull = cull;
        let clamped = self.weight.max(cull).min(1.0 - cull);
        let t = (clamped - cull) / (1.0 - 2.0 * cull);
        let square = t * t;
        // At the ends of the range the weight eases (smoothstep halves) towards the outer child.
        let weight = if t <= 0.5 && self.left == self.order[0] {
            (4.0 - 4.0 * t) * square
        } else if t > 0.5 && self.right == *self.order.last().unwrap() {
            let four = t * 4.0;
            (8.0 - four) * square - four + 1.0
        } else {
            self.weight
        };
        let (l, r) = (self.left, self.right);
        let left = self.children[l].evaluate(cull, enabled && weight < 1.0, update_history, out);
        let right = self.children[r].evaluate(cull, enabled && weight > 0.0, update_history, out);
        if left && right {
            out.push(PoseCommand::Blend { weight });
        }
        left || right
    }
}

/// Children weighted by barycentric coordinates in the simplex that holds the parameters.
#[derive(Clone, Debug)]
pub struct BlendSpace {
    pub parameters: Vec<NameId>,
    pub children: Vec<Playback>,
    pub simplexes: Vec<SimplexDef>,
    values: Vec<f32>,
    pub current: usize,
    pub weights: Vec<f32>,
    pub time: f32,
}

/// Signed distances to the planes opposite each vertex, scaled to 1 at the vertex.
pub fn simplex_coordinates(s: &SimplexDef, point: &[f32]) -> Vec<f32> {
    let n = s.children.len();
    (0..n)
        .map(|i| {
            let anchor = &s.vertices[(i + 1) % n];
            let dot: f32 = (0..point.len())
                .map(|j| s.normals[i][j] * (point[j] - anchor[j]))
                .sum();
            dot * s.scales[i]
        })
        .collect()
}

/// The point projected into the simplex by successive coordinate slices (SK8 Simplex::project).
fn simplex_project(s: &SimplexDef, point: &[f32]) -> Vec<f32> {
    let mut matrix = s.vertices.clone();
    let mut projected = Vec::with_capacity(point.len());
    for &target in point {
        let count = matrix.len() - 1;
        let mut sliced: Vec<Vec<f32>> = Vec::new();
        'edges: for i in 0..count {
            for j in i + 1..matrix.len() {
                if sliced.len() == count {
                    break 'edges;
                }
                let (a, b) = (&matrix[i], &matrix[j]);
                if (a[0] < target) == (b[0] < target) {
                    continue;
                }
                let delta = b[0] - a[0];
                if delta.abs() <= 1e-5 {
                    sliced.push(a[1..].to_vec());
                    if sliced.len() < count {
                        sliced.push(b[1..].to_vec());
                    }
                } else {
                    let t = (target - a[0]) / delta;
                    sliced.push(
                        a[1..]
                            .iter()
                            .zip(&b[1..])
                            .map(|(&a, &b)| (b - a) * t + a)
                            .collect(),
                    );
                }
            }
        }
        if sliced.is_empty() {
            let nearest = (0..matrix.len())
                .min_by(|&x, &y| {
                    let dx = (matrix[x][0] - target) * (matrix[x][0] - target);
                    let dy = (matrix[y][0] - target) * (matrix[y][0] - target);
                    dx.partial_cmp(&dy).unwrap_or(core::cmp::Ordering::Equal)
                })
                .unwrap_or(0);
            projected.extend_from_slice(&matrix[nearest]);
            break;
        }
        projected.push(target);
        while sliced.len() < count {
            let last = sliced.last().unwrap().clone();
            sliced.push(last);
        }
        matrix = sliced;
    }
    projected
}

fn normalize(w: &mut [f32]) -> bool {
    let sum: f32 = w.iter().sum();
    if !sum.is_finite() || sum <= 0.0 {
        return false;
    }
    w.iter_mut().for_each(|v| *v /= sum);
    true
}

impl BlendSpace {
    fn new(
        parameters: Vec<NameId>,
        children: Vec<Playback>,
        simplexes: Vec<SimplexDef>,
    ) -> Option<Self> {
        let d = parameters.len();
        let ok = d > 0
            && d <= 4
            && children.len() > d
            && !simplexes.is_empty()
            && simplexes.iter().all(|s| {
                s.children.len() == d + 1
                    && s.vertices.len() == d + 1
                    && s.normals.len() == d + 1
                    && s.scales.len() == d + 1
                    && s.children.iter().all(|&c| (c as usize) < children.len())
                    && s.vertices.iter().chain(&s.normals).all(|v| v.len() == d)
            });
        if !ok {
            return None;
        }
        let mut weights = vec![0.0; d + 1];
        weights[0] = 1.0;
        Some(BlendSpace {
            parameters,
            children,
            simplexes,
            values: vec![0.0; d],
            current: 0,
            weights,
            time: 0.0,
        })
    }
    /// (child index, weight) of the playing simplex.
    pub fn active_weights(&self) -> impl Iterator<Item = (usize, f32)> + '_ {
        self.simplexes[self.current]
            .children
            .iter()
            .map(|&c| c as usize)
            .zip(self.weights.iter().copied())
    }
    fn length(&self) -> f32 {
        self.active_weights()
            .map(|(i, w)| w * self.children[i].length())
            .sum()
    }
    fn set_time(&mut self, time: f32) {
        let phase = time / self.length();
        for &i in &self.simplexes[self.current].children {
            let c = &mut self.children[i as usize];
            let l = c.length();
            c.set_time(phase * l);
        }
    }
    fn advance(&mut self, dt: f32, phase: f32, result: &mut Advance) {
        let length = self.length();
        let first = self.simplexes[self.current].children[0] as usize;
        let first_length = self.children[first].length();
        for slot in 0..self.simplexes[self.current].children.len() {
            let i = self.simplexes[self.current].children[slot] as usize;
            let c = &mut self.children[i];
            let mut r = Advance::default();
            let l = c.length();
            c.advance(l / length * dt, phase, &mut r);
            if slot == 0 {
                *result = r;
            }
        }
        let scale = length / first_length;
        result.overshoot *= scale;
        result.remaining_before_wrap *= scale;
        self.time = self.children[first].time() * scale;
    }
    fn set_parameters(&mut self, params: &[Param]) -> bool {
        let mut changed = false;
        for p in params {
            if let Some(i) = self.parameters.iter().position(|&n| n == p.name)
                && p.value.is_finite()
            {
                self.values[i] = p.value;
                changed = true;
            }
        }
        if !changed {
            return false;
        }
        let mut selected = None;
        let mut closest = f32::MAX;
        for (index, s) in self.simplexes.iter().enumerate() {
            let w = simplex_coordinates(s, &self.values);
            if w.iter().all(|&w| w >= 0.0) {
                selected = Some((index, w));
                break;
            }
            let mut w = simplex_coordinates(s, &simplex_project(s, &self.values));
            w.iter_mut().for_each(|v| *v = v.max(0.0));
            if !normalize(&mut w) {
                continue;
            }
            let distance: f32 = self
                .values
                .iter()
                .enumerate()
                .map(|(j, &target)| {
                    let p: f32 = s.vertices.iter().zip(&w).map(|(v, &w)| v[j] * w).sum();
                    (p - target) * (p - target)
                })
                .sum();
            if distance < closest {
                closest = distance;
                selected = Some((index, w));
            }
        }
        let Some((index, mut w)) = selected else {
            return false;
        };
        w.iter_mut().for_each(|v| *v = v.clamp(0.0, 1.0));
        if !normalize(&mut w) {
            return false;
        }
        if index != self.current {
            let old = &self.children[self.simplexes[self.current].children[0] as usize];
            let phase = old.time() / old.length();
            for &i in &self.simplexes[index].children {
                let c = &mut self.children[i as usize];
                let l = c.length();
                c.set_time(phase * l);
            }
            self.current = index;
        }
        self.weights = w;
        true
    }
    fn query_attribute(&self, db: &MotionDb, name: NameId, mask: u32, out: &mut Attr) -> bool {
        // As the original: the first d + 1 authored children, not the active simplex's.
        if !self.children[0].query_attribute(db, name, mask, out) {
            return false;
        }
        out.scale(self.weights[0]);
        for i in 1..self.weights.len() {
            let mut r = Attr::scalar(name, 0.0);
            if !self.children[i].query_attribute(db, name, 15, &mut r) {
                return false;
            }
            out.add_weighted(&r, self.weights[i]);
        }
        true
    }
}

/// The candidate nearest the parameters, chosen the first time all of them are known, then kept.
#[derive(Clone, Debug)]
pub struct SelectionSpace {
    pub parameters: Vec<SelectionParam>,
    pub candidates: Vec<(Node, Vec<f32>, Playback)>,
    pub selected: Option<usize>,
    values: Vec<Option<f32>>,
    speed: f32,
}

/// SK8 selection_space::distance: weighted squared differences, normalised by each parameter's range.
pub fn selection_distance(params: &[SelectionParam], values: &[f32], candidate: &[f32]) -> f32 {
    params
        .iter()
        .zip(values.iter().zip(candidate))
        .map(|(p, (&v, &c))| {
            let d = selection_delta(p, v, c);
            p.weight * d * d
        })
        .sum()
}

fn selection_delta(p: &SelectionParam, mut value: f32, mut candidate: f32) -> f32 {
    let range = p.maximum - p.minimum;
    if range > 1.0 / 65536.0 {
        candidate /= range;
        value /= range;
    }
    match p.mode {
        1 if value > candidate => (1.0 - value) + candidate,
        2 => {
            let difference = (value - candidate).abs();
            difference.min(1.0 - difference)
        }
        _ => candidate - value,
    }
}

impl SelectionSpace {
    fn new(parameters: Vec<SelectionParam>, candidates: Vec<(Node, Vec<f32>, Playback)>) -> Self {
        let n = parameters.len();
        SelectionSpace {
            parameters,
            candidates,
            selected: None,
            values: vec![None; n],
            speed: 1.0,
        }
    }
    pub fn current(&self) -> Option<&Playback> {
        self.selected.map(|i| &self.candidates[i].2)
    }
    pub fn current_mut(&mut self) -> Option<&mut Playback> {
        self.selected.map(|i| &mut self.candidates[i].2)
    }
    fn set_parameters(&mut self, db: &MotionDb, params: &[Param]) -> bool {
        for p in params {
            for (q, v) in self.parameters.iter().zip(&mut self.values) {
                if q.name == p.name {
                    *v = Some(p.value);
                }
            }
        }
        if self.selected.is_none()
            && let Some(values) = self.values.iter().copied().collect::<Option<Vec<f32>>>()
        {
            let mut best = f32::MAX;
            let mut chosen = None;
            for (i, c) in self.candidates.iter().enumerate() {
                let d = selection_distance(&self.parameters, &values, &c.1);
                if d < best {
                    best = d;
                    chosen = Some(i);
                }
            }
            // The first candidate with the chosen name.
            self.selected = chosen.and_then(|c| {
                self.candidates
                    .iter()
                    .position(|x| x.0 == self.candidates[c].0)
            });
        }
        let speed = self.speed;
        if let Some(c) = self.current_mut() {
            c.set_parameters(db, params);
            c.set_speed(speed);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::tests::{Lib, lib};

    fn param(db: &MotionDb, name: &str, value: f32) -> Param {
        Param {
            name: db.name_id(name).unwrap(),
            value,
            normalized: false,
        }
    }

    #[test]
    fn clip_clock_loops_and_reports_the_step() {
        let bytes = lib();
        let db = MotionDb::parse(&bytes).unwrap();
        let mut p = Playback::build(&db, "LOOP_A", &[]).unwrap();
        // 31 frames at 30 fps: 1 s long.
        assert!((p.length() - 1.0).abs() < 1e-6);
        let mut r = Advance::default();
        p.advance(0.75, 0.0, &mut r);
        p.advance(0.5, 0.0, &mut r);
        assert!(r.crossed_end);
        assert!((p.time() - 0.25).abs() < 1e-5);
        let mut out = Vec::new();
        assert!(p.evaluate(0.01, true, true, &mut out));
        assert_eq!(out.len(), 1);
        let PoseCommand::Clip { loops, time, .. } = out[0] else {
            panic!()
        };
        assert_eq!(loops, 1, "one wrap since the last evaluation");
        assert!((time - 0.25).abs() < 1e-5);
    }

    #[test]
    fn phase_blend_picks_neighbours_by_the_childrens_own_values() {
        let bytes = lib();
        let db = MotionDb::parse(&bytes).unwrap();
        let mut p = Playback::build(&db, Lib::PHASE, &[]).unwrap();
        // Children's SPEED attributes: LOOP_A 2, LOOP_B 6, SHORT 4.
        assert!(p.set_parameters(&db, &[param(&db, "SPEED", 5.0)]));
        let Playback::PhaseBlend(t) = &p else {
            panic!()
        };
        let names: Vec<&str> = [t.left, t.right]
            .iter()
            .map(|&i| match &t.children[i] {
                Playback::Clip { clip, .. } => db.name(db.clip(*clip).name),
                _ => "",
            })
            .collect();
        assert_eq!(names, ["SHORT", "LOOP_B"]);
        assert!((t.weight - 0.5).abs() < 1e-6);
        // Length is the weighted mix of the two (0.5 s and 2 s).
        assert!((p.length() - 1.25).abs() < 1e-5);
        let mut out = Vec::new();
        assert!(p.evaluate(0.01, true, true, &mut out));
        let leaves = leaf_weights(&out);
        assert_eq!(leaves.len(), 2);
        assert!((leaves.iter().map(|l| l.2).sum::<f32>() - 1.0).abs() < 1e-5);
        // Clocks stay in phase: half way through the blend is half way through both clips.
        let mut r = Advance::default();
        let l = p.length();
        p.advance(l * 0.5, 0.0, &mut r);
        let Playback::PhaseBlend(t) = &p else {
            panic!()
        };
        for i in [t.left, t.right] {
            let c = &t.children[i];
            assert!((c.time() / c.length() - 0.5).abs() < 1e-4);
        }
        // Past the top of the range: all of the fastest child (normalised 1.0 also gets there).
        p.set_parameters(&db, &[param(&db, "SPEED", 9.0)]);
        let Playback::PhaseBlend(t) = &p else {
            panic!()
        };
        assert_eq!(t.weight, 1.0);
        let mut a = Attr::scalar(db.name_id("SPEED").unwrap(), 0.0);
        assert!(p.query_attribute(&db, db.name_id("SPEED").unwrap(), 15, &mut a));
        assert!((a.value() - 6.0).abs() < 1e-5, "{}", a.value());
    }

    #[test]
    fn blend_space_weights_are_barycentric_and_projected_outside() {
        let bytes = lib();
        let db = MotionDb::parse(&bytes).unwrap();
        let mut p = Playback::build(&db, Lib::SPACE, &[]).unwrap();
        p.set_parameters(&db, &[param(&db, "X", 0.25), param(&db, "Y", 0.5)]);
        let Playback::BlendSpace(t) = &p else {
            panic!()
        };
        let w: Vec<f32> = t.weights.clone();
        assert!(
            (w[0] - 0.25).abs() < 1e-5 && (w[1] - 0.25).abs() < 1e-5 && (w[2] - 0.5).abs() < 1e-5
        );
        let mut out = Vec::new();
        assert!(p.evaluate(0.01, true, false, &mut out));
        assert_eq!(out.len(), 4);
        assert!(matches!(
            out[3],
            PoseCommand::WeightedBlend { count: 3, .. }
        ));
        // Outside the triangle: clamped onto it, weights still sum to 1 and none negative.
        p.set_parameters(&db, &[param(&db, "X", 2.0), param(&db, "Y", 0.5)]);
        let Playback::BlendSpace(t) = &p else {
            panic!()
        };
        assert!(t.weights.iter().all(|&w| w >= 0.0));
        assert!((t.weights.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!(t.weights[1] > 0.99, "{:?}", t.weights);
    }

    #[test]
    fn selector_uses_the_construction_value_and_selection_space_latches() {
        let bytes = lib();
        let db = MotionDb::parse(&bytes).unwrap();
        let key = db.name_id("STANCE").unwrap();
        let goofy = db.name_id("GOOFY").unwrap();
        let clip_of = |p: &Playback| match p {
            Playback::Clip { clip, .. } => db.name(db.clip(*clip).name),
            _ => "tree",
        };
        let p = Playback::build(&db, Lib::SELECT, &[(key, goofy)]).unwrap();
        assert_eq!(clip_of(&p), "LOOP_B");
        let p = Playback::build(&db, Lib::SELECT, &[]).unwrap();
        assert_eq!(
            clip_of(&p),
            "LOOP_A",
            "default without a construction value"
        );

        let mut p = Playback::build(&db, Lib::CHOOSE, &[]).unwrap();
        let mut out = Vec::new();
        assert!(
            !p.evaluate(0.01, true, true, &mut out),
            "nothing chosen yet"
        );
        p.set_parameters(&db, &[param(&db, "ANGLE", 170.0)]);
        let Playback::SelectionSpace(t) = &p else {
            panic!()
        };
        assert_eq!(clip_of(t.current().unwrap()), "SHORT");
        // Latched: a new value doesn't change the choice.
        p.set_parameters(&db, &[param(&db, "ANGLE", 0.0)]);
        let Playback::SelectionSpace(t) = &p else {
            panic!()
        };
        assert_eq!(clip_of(t.current().unwrap()), "SHORT");
        assert!(p.evaluate(0.01, true, true, &mut out));
    }

    #[test]
    fn missing_children_fail_to_build() {
        let bytes = lib();
        let db = MotionDb::parse(&bytes).unwrap();
        assert!(matches!(
            Playback::build(&db, Lib::BROKEN, &[]),
            Err(BuildError::Missing(_))
        ));
        assert!(Playback::build(&db, "NOPE", &[]).is_err());
    }
}
