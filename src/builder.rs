use crate::errors::{
    AutomatonScaleError, DaachorseError, DuplicatePatternError, InvalidArgumentError,
    PatternScaleError,
};
use crate::{DoubleArrayAhoCorasick, Output, State, OUTPOS_INVALID};

// The length of each double-array block.
const BLOCK_LEN: usize = 256;
// The number of last blocks to be searched in `DoubleArrayAhoCorasickBuilder::find_base`.
const FREE_BLOCKS: usize = 16;
// The number of last states (or elements) to be searched in `DoubleArrayAhoCorasickBuilder::find_base`.
const FREE_STATES: usize = BLOCK_LEN * FREE_BLOCKS;
// The maximum state index used as an invalid value.
const STATE_IDX_INVALID: u32 = std::u32::MAX;
// The maximum value of a pattern used as an invalid value.
const VALUE_INVALID: u32 = std::u32::MAX;
// The maximum length of a pattern used as an invalid value.
const LENGTH_INVALID: u32 = std::u32::MAX >> 1;
// The maximum FAIL value.
const FAIL_MAX: usize = 0x00ffffff;

struct SparseTrie {
    states: Vec<Vec<(u8, usize)>>,
    outputs: Vec<(u32, u32)>,
    len: usize,
}

impl SparseTrie {
    fn new() -> Self {
        Self {
            states: vec![vec![]],
            outputs: vec![(VALUE_INVALID, LENGTH_INVALID)],
            len: 0,
        }
    }

    #[inline(always)]
    fn add(&mut self, pattern: &[u8], value: u32) -> Result<(), DaachorseError> {
        if value == VALUE_INVALID {
            let e = PatternScaleError {
                msg: format!("Input value must be < {}", VALUE_INVALID),
            };
            return Err(DaachorseError::PatternScale(e));
        }
        if pattern.len() >= LENGTH_INVALID as usize {
            let e = PatternScaleError {
                msg: format!("Pattern length must be < {}", LENGTH_INVALID),
            };
            return Err(DaachorseError::PatternScale(e));
        }

        let mut state_id = 0;
        for &c in pattern {
            state_id = self.get(state_id, c).unwrap_or_else(|| {
                let next_state_id = self.states.len();
                self.states.push(vec![]);
                self.states[state_id].push((c, next_state_id));
                self.outputs.push((VALUE_INVALID, LENGTH_INVALID));
                next_state_id
            });
        }

        let output = self.outputs.get_mut(state_id).unwrap();
        if output.0 != VALUE_INVALID {
            let e = DuplicatePatternError {
                pattern: pattern.to_vec(),
            };
            return Err(DaachorseError::DuplicatePattern(e));
        }
        *output = (value, pattern.len() as u32);
        self.len += 1;
        Ok(())
    }

    #[inline(always)]
    fn get(&self, state_id: usize, c: u8) -> Option<usize> {
        for &(cc, child_id) in &self.states[state_id] {
            if c == cc {
                return Some(child_id);
            }
        }
        None
    }
}

// TODO: Optimize in memory
#[derive(Clone, Copy)]
struct Extra {
    // For double-array construction
    used_base: bool,
    used_index: bool,
    next: usize,
    prev: usize,
    // For output construction
    output: (u32, u32),
    processed: bool,
}

impl Default for Extra {
    fn default() -> Self {
        Self {
            used_base: false,
            used_index: false,
            next: std::usize::MAX,
            prev: std::usize::MAX,
            output: (VALUE_INVALID, LENGTH_INVALID),
            processed: false,
        }
    }
}

#[derive(Clone, Copy)]
struct StatePair {
    da_idx: usize,
    st_idx: usize,
}

/// Builder of [`DoubleArrayAhoCorasick`].
pub struct DoubleArrayAhoCorasickBuilder {
    states: Vec<State>,
    outputs: Vec<Output>,
    extras: Vec<Extra>,
    visits: Vec<StatePair>,
    head_idx: usize,
}

impl DoubleArrayAhoCorasickBuilder {
    /// Creates a new [`DoubleArrayAhoCorasickBuilder`].
    ///
    /// # Arguments
    ///
    /// * `init_size` - Initial size of the Double-Array (<= 2^{32}).
    ///
    /// # Errors
    ///
    /// [`DaachorseError`] is returned when invalid arguements are given.
    ///
    /// # Examples
    ///
    /// ```
    /// use daachorse::DoubleArrayAhoCorasickBuilder;
    ///
    /// let builder = DoubleArrayAhoCorasickBuilder::new(16).unwrap();
    ///
    /// let patterns = vec!["bcd", "ab", "a"];
    /// let pma = builder.build(patterns).unwrap();
    ///
    /// let mut it = pma.find_iter("abcd");
    ///
    /// let m = it.next().unwrap();
    /// assert_eq!((0, 1, 2), (m.start(), m.end(), m.value()));
    ///
    /// let m = it.next().unwrap();
    /// assert_eq!((1, 4, 0), (m.start(), m.end(), m.value()));
    ///
    /// assert_eq!(None, it.next());
    /// ```
    pub fn new(init_size: usize) -> Result<Self, DaachorseError> {
        if init_size > STATE_IDX_INVALID as usize {
            let e = InvalidArgumentError {
                arg: "init_size",
                msg: format!("must be <= {}", STATE_IDX_INVALID),
            };
            return Err(DaachorseError::InvalidArgument(e));
        }

        let init_capa = std::cmp::min(BLOCK_LEN, init_size / BLOCK_LEN * BLOCK_LEN);
        Ok(Self {
            states: Vec::with_capacity(init_capa),
            outputs: vec![],
            extras: Vec::with_capacity(init_capa),
            visits: vec![],
            head_idx: std::usize::MAX,
        })
    }

    /// Builds and returns a new [`DoubleArrayAhoCorasick`] from input patterns.
    /// The value `i` is automatically associated with `patterns[i]`.
    ///
    /// # Arguments
    ///
    /// * `patterns` - List of patterns.
    ///
    /// # Errors
    ///
    /// [`DaachorseError`] is returned when
    ///   - the `patterns` contains duplicate entries,
    ///   - the scale of `patterns` exceeds the expected one, or
    ///   - the scale of the resulting automaton exceeds the expected one.
    ///
    /// # Examples
    ///
    /// ```
    /// use daachorse::DoubleArrayAhoCorasickBuilder;
    ///
    /// let builder = DoubleArrayAhoCorasickBuilder::new(16).unwrap();
    ///
    /// let patterns = vec!["bcd", "ab", "a"];
    /// let pma = builder.build(patterns).unwrap();
    ///
    /// let mut it = pma.find_iter("abcd");
    ///
    /// let m = it.next().unwrap();
    /// assert_eq!((0, 1, 2), (m.start(), m.end(), m.value()));
    ///
    /// let m = it.next().unwrap();
    /// assert_eq!((1, 4, 0), (m.start(), m.end(), m.value()));
    ///
    /// assert_eq!(None, it.next());
    /// ```
    pub fn build<I, P>(mut self, patterns: I) -> Result<DoubleArrayAhoCorasick, DaachorseError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<[u8]>,
    {
        let patvals = patterns.into_iter().enumerate().map(|(i, p)| (p, i as u32));
        let sparse_trie = self.build_sparse_trie(patvals)?;
        self.build_double_array(&sparse_trie)?;
        self.add_fails(&sparse_trie)?;
        self.build_outputs()?;
        self.set_dummy_outputs();

        let DoubleArrayAhoCorasickBuilder {
            mut states,
            mut outputs,
            ..
        } = self;

        states.shrink_to_fit();
        outputs.shrink_to_fit();

        Ok(DoubleArrayAhoCorasick { states, outputs })
    }

    /// Builds and returns a new [`DoubleArrayAhoCorasick`] from input pattern-value pairs.
    ///
    /// # Arguments
    ///
    /// * `patvals` - List of pattern-value pairs, where the value is of type `u32` and less than `u32::MAX`.
    ///
    /// # Errors
    ///
    /// [`DaachorseError`] is returned when
    ///   - the `patvals` contains duplicate patterns,
    ///   - the `patvals` contains invalid values,
    ///   - the scale of `patvals` exceeds the expected one, or
    ///   - the scale of the resulting automaton exceeds the expected one.
    ///
    /// # Examples
    ///
    /// ```
    /// use daachorse::DoubleArrayAhoCorasickBuilder;
    ///
    /// let builder = DoubleArrayAhoCorasickBuilder::new(16).unwrap();
    ///
    /// let patvals = vec![("bcd", 0), ("ab", 1), ("a", 2), ("e", 1)];
    /// let pma = builder.build_with_values(patvals).unwrap();
    ///
    /// let mut it = pma.find_iter("abcde");
    ///
    /// let m = it.next().unwrap();
    /// assert_eq!((0, 1, 2), (m.start(), m.end(), m.value()));
    ///
    /// let m = it.next().unwrap();
    /// assert_eq!((1, 4, 0), (m.start(), m.end(), m.value()));
    ///
    /// let m = it.next().unwrap();
    /// assert_eq!((4, 5, 1), (m.start(), m.end(), m.value()));
    ///
    /// assert_eq!(None, it.next());
    /// ```
    pub fn build_with_values<I, P>(
        mut self,
        patvals: I,
    ) -> Result<DoubleArrayAhoCorasick, DaachorseError>
    where
        I: IntoIterator<Item = (P, u32)>,
        P: AsRef<[u8]>,
    {
        let sparse_trie = self.build_sparse_trie(patvals)?;
        self.build_double_array(&sparse_trie)?;
        self.add_fails(&sparse_trie)?;
        self.build_outputs()?;
        self.set_dummy_outputs();

        let DoubleArrayAhoCorasickBuilder {
            mut states,
            mut outputs,
            ..
        } = self;

        states.shrink_to_fit();
        outputs.shrink_to_fit();

        Ok(DoubleArrayAhoCorasick { states, outputs })
    }

    fn build_sparse_trie<I, P>(&mut self, patvals: I) -> Result<SparseTrie, DaachorseError>
    where
        I: IntoIterator<Item = (P, u32)>,
        P: AsRef<[u8]>,
    {
        let mut trie = SparseTrie::new();
        for (pattern, value) in patvals {
            trie.add(pattern.as_ref(), value)?;
        }
        Ok(trie)
    }

    fn build_double_array(&mut self, sparse_trie: &SparseTrie) -> Result<(), DaachorseError> {
        let mut state_id_map = vec![std::usize::MAX; sparse_trie.states.len()];
        state_id_map[0] = 0;

        self.init_array();

        for (i, edges) in sparse_trie.states.iter().enumerate() {
            let idx = state_id_map[i];
            self.extras[idx].output = sparse_trie.outputs[i];

            if edges.is_empty() {
                continue;
            }

            let base = self.find_base(edges);
            if base >= self.states.len() {
                self.extend_array()?;
            }

            for &(c, child_id) in edges {
                let child_idx = base ^ c as usize;
                self.fix_state(child_idx);
                self.states[child_idx].set_check(c);
                state_id_map[child_id] = child_idx;
            }
            self.states[idx].set_base(base as u32);
            self.extras[base].used_base = true;
        }

        // If the root block has not been closed, it has to be closed for setting CHECK[0] to a valid value.
        if self.states.len() <= FREE_STATES {
            self.close_block(0);
        }

        while self.head_idx != std::usize::MAX {
            let block_idx = self.head_idx / BLOCK_LEN;
            self.close_block(block_idx);
        }
        Ok(())
    }

    fn init_array(&mut self) {
        self.states.resize(BLOCK_LEN, Default::default());
        self.extras.resize(BLOCK_LEN, Default::default());
        self.head_idx = 0;

        for i in 0..BLOCK_LEN {
            if i == 0 {
                self.extras[i].prev = BLOCK_LEN - 1;
            } else {
                self.extras[i].prev = i - 1;
            }
            if i == BLOCK_LEN - 1 {
                self.extras[i].next = 0;
            } else {
                self.extras[i].next = i + 1;
            }
        }

        self.states[0].set_check(0);
        self.fix_state(0);
    }

    fn fix_state(&mut self, i: usize) {
        debug_assert!(!self.extras[i].used_index);
        self.extras[i].used_index = true;

        let next = self.extras[i].next;
        let prev = self.extras[i].prev;
        self.extras[prev].next = next;
        self.extras[next].prev = prev;

        if self.head_idx == i {
            if next == i {
                self.head_idx = std::usize::MAX;
            } else {
                self.head_idx = next;
            }
        }
    }

    #[inline(always)]
    fn find_base(&self, edges: &[(u8, usize)]) -> usize {
        if self.head_idx == std::usize::MAX {
            return self.states.len();
        }
        let mut idx = self.head_idx;
        loop {
            debug_assert!(!self.extras[idx].used_index);
            let base = idx ^ edges[0].0 as usize;
            if self.check_valid_base(base, edges) {
                return base;
            }
            idx = self.extras[idx].next;
            if idx == self.head_idx {
                break;
            }
        }
        self.states.len()
    }

    fn check_valid_base(&self, base: usize, edges: &[(u8, usize)]) -> bool {
        if self.extras[base].used_base {
            return false;
        }
        for &(c, _) in edges {
            let idx = base ^ c as usize;
            if self.extras[idx].used_index {
                return false;
            }
        }
        true
    }

    fn extend_array(&mut self) -> Result<(), DaachorseError> {
        let old_len = self.states.len();
        let new_len = old_len + BLOCK_LEN;

        if new_len > STATE_IDX_INVALID as usize {
            let e = AutomatonScaleError {
                msg: format!("states.len() must be <= {}", STATE_IDX_INVALID),
            };
            return Err(DaachorseError::AutomatonScale(e));
        }

        for i in old_len..new_len {
            self.states.push(Default::default());
            self.extras.push(Default::default());
            self.extras[i].next = i + 1;
            self.extras[i].prev = i - 1;
        }

        if self.head_idx == std::usize::MAX {
            self.extras[old_len].prev = new_len - 1;
            self.extras[new_len - 1].next = old_len;
            self.head_idx = old_len;
        } else {
            let tail_idx = self.extras[self.head_idx].prev;
            self.extras[old_len].prev = tail_idx;
            self.extras[tail_idx].next = old_len;
            self.extras[new_len - 1].next = self.head_idx;
            self.extras[self.head_idx].prev = new_len - 1;
        }

        if FREE_STATES <= old_len {
            self.close_block((old_len - FREE_STATES) / BLOCK_LEN);
        }

        Ok(())
    }

    /// Note: Assumes all the previous blocks are closed.
    fn close_block(&mut self, block_idx: usize) {
        let beg_idx = block_idx * BLOCK_LEN;
        let end_idx = beg_idx + BLOCK_LEN;

        if block_idx == 0 || self.head_idx < end_idx {
            self.remove_invalid_checks(block_idx);
        }
        while self.head_idx < end_idx && self.head_idx != std::usize::MAX {
            self.fix_state(self.head_idx);
        }
    }

    fn remove_invalid_checks(&mut self, block_idx: usize) {
        let beg_idx = block_idx * BLOCK_LEN;
        let end_idx = beg_idx + BLOCK_LEN;

        let unused_base = {
            let mut i = beg_idx;
            while i < end_idx {
                if !self.extras[i].used_base {
                    break;
                }
                i += 1;
            }
            i
        };
        debug_assert_ne!(unused_base, end_idx);

        for c in 0..BLOCK_LEN {
            let idx = unused_base ^ c;
            if idx == 0 || !self.extras[idx].used_index {
                self.states[idx].set_check(c as u8);
            }
        }
    }

    fn add_fails(&mut self, sparse_trie: &SparseTrie) -> Result<(), DaachorseError> {
        self.states[0].set_fail(0);
        self.visits.reserve(sparse_trie.states.len());

        for &(c, st_child_idx) in &sparse_trie.states[0] {
            let da_child_idx = self.get_child_index(0, c).unwrap();
            self.states[da_child_idx].set_fail(0);
            self.visits.push(StatePair {
                da_idx: da_child_idx,
                st_idx: st_child_idx,
            });
        }

        let mut vi = 0;
        while vi < self.visits.len() {
            let StatePair {
                da_idx: da_state_idx,
                st_idx: st_state_idx,
            } = self.visits[vi];
            vi += 1;

            for &(c, st_child_idx) in &sparse_trie.states[st_state_idx] {
                let da_child_idx = self.get_child_index(da_state_idx, c).unwrap();
                let mut fail_idx = self.states[da_state_idx].fail() as usize;
                let new_fail_idx = loop {
                    if let Some(child_fail_idx) = self.get_child_index(fail_idx, c) {
                        break child_fail_idx;
                    }
                    let next_fail_idx = self.states[fail_idx].fail() as usize;
                    if fail_idx == 0 && next_fail_idx == 0 {
                        break 0;
                    }
                    fail_idx = next_fail_idx;
                };
                if new_fail_idx > FAIL_MAX {
                    let e = AutomatonScaleError {
                        msg: format!("fail_idx must be <= {}", FAIL_MAX),
                    };
                    return Err(DaachorseError::AutomatonScale(e));
                }

                self.states[da_child_idx].set_fail(new_fail_idx as u32);
                self.visits.push(StatePair {
                    da_idx: da_child_idx,
                    st_idx: st_child_idx,
                });
            }
        }

        Ok(())
    }

    fn build_outputs(&mut self) -> Result<(), DaachorseError> {
        let error_checker = |outputs: &Vec<Output>| {
            if outputs.len() > OUTPOS_INVALID as usize {
                let e = AutomatonScaleError {
                    msg: format!("outputs.len() must be <= {}", OUTPOS_INVALID),
                };
                Err(DaachorseError::AutomatonScale(e))
            } else {
                Ok(())
            }
        };

        for sp in self.visits.iter().rev() {
            let mut da_state_idx = sp.da_idx;

            let Extra {
                output, processed, ..
            } = self.extras[da_state_idx];

            if output.0 == VALUE_INVALID {
                continue;
            }
            if processed {
                debug_assert!(self.states[da_state_idx].output_pos().is_some());
                continue;
            }
            debug_assert!(self.states[da_state_idx].output_pos().is_none());

            self.extras[da_state_idx].processed = true;
            self.states[da_state_idx].set_output_pos(self.outputs.len() as u32);
            self.outputs.push(Output::new(output.0, output.1, true));

            error_checker(&self.outputs)?;

            loop {
                da_state_idx = self.states[da_state_idx].fail() as usize;
                if da_state_idx == 0 {
                    break;
                }

                let Extra {
                    output, processed, ..
                } = self.extras[da_state_idx];

                if output.0 == VALUE_INVALID {
                    continue;
                }

                if processed {
                    let mut clone_pos = self.states[da_state_idx].output_pos().unwrap() as usize;
                    debug_assert!(!self.outputs[clone_pos].is_begin());
                    while !self.outputs[clone_pos].is_begin() {
                        self.outputs.push(self.outputs[clone_pos]);
                        clone_pos += 1;
                    }
                    error_checker(&self.outputs)?;
                    break;
                }

                self.extras[da_state_idx].processed = true;
                self.states[da_state_idx].set_output_pos(self.outputs.len() as u32);
                self.outputs.push(Output::new(output.0, output.1, false));
            }
        }

        // sentinel
        self.outputs
            .push(Output::new(VALUE_INVALID, LENGTH_INVALID, true));
        error_checker(&self.outputs)?;

        Ok(())
    }

    fn set_dummy_outputs(&mut self) {
        for sp in self.visits.iter() {
            let da_state_idx = sp.da_idx;

            let Extra {
                output, processed, ..
            } = self.extras[da_state_idx];

            if processed {
                debug_assert!(self.states[da_state_idx].output_pos().is_some());
                continue;
            }
            debug_assert!(self.states[da_state_idx].output_pos().is_none());
            debug_assert_eq!(output.0, VALUE_INVALID);

            let fail_idx = self.states[da_state_idx].fail() as usize;
            if let Some(output_pos) = self.states[fail_idx].output_pos() {
                self.states[da_state_idx].set_output_pos(output_pos);
            }
        }
    }

    #[inline(always)]
    fn get_child_index(&self, idx: usize, c: u8) -> Option<usize> {
        self.states[idx].base().and_then(|base| {
            let child_idx = (base ^ c as u32) as usize;
            Some(child_idx).filter(|&x| self.states[x].check() == c)
        })
    }
}
