//! Logic modules: count and route triggers for arrangement (FR-LIB-22,
//! FR-LIB-27, ADR-0012).
//!
//! Inputs count rising edges, and trigger outputs are high for one sample
//! (ADR-0007), so a logic module never misses or merges a trigger.

use loom_core::signal::{EdgeDetector, trigger};
use loom_core::{
    Module, ModuleDescriptor, ModuleRole, Panel, ParamOptions, ParamSpec, Polarity, PortDefault, PortIo, PortSpec,
    ProcessContext, SignalType, Widget,
};

/// `loom.logic.divide`: fires on every Nth incoming trigger.
///
/// Counting starts again at each reset. It fires on the triggers numbered
/// N, 2N, 3N after a reset, and `offset` moves each firing that many
/// triggers earlier. Offset N-1 fires on the first trigger. An offset of N
/// or more counts as N-1.
pub static DIVIDE: ModuleDescriptor = ModuleDescriptor {
    id: "loom.logic.divide",
    version: 1,
    name: "Clock Divide",
    category: "Logic",
    description: "Fires on every Nth trigger. Offset fires earlier in the count. Reset starts the count again.",
    inputs: &[
        PortSpec {
            name: "trigger",
            label: "Trigger",
            signal: SignalType::Gate,
            description: "The triggers to count.",
            default: PortDefault::Value(0.0),
            polarity: Polarity::Unipolar,
        },
        PortSpec {
            name: "reset",
            label: "Reset",
            signal: SignalType::Gate,
            description: "Starts the count again.",
            default: PortDefault::Value(0.0),
            polarity: Polarity::Unipolar,
        },
    ],
    outputs: &[PortSpec {
        name: "trigger",
        label: "Trigger",
        signal: SignalType::Gate,
        description: "Fires on every Nth trigger.",
        default: PortDefault::Value(0.0),
        polarity: Polarity::Unipolar,
    }],
    params: &[
        ParamSpec {
            name: "n",
            label: "Division",
            description: "Fires on every Nth trigger.",
            min: 1.0,
            max: 64.0,
            default: 2.0,
            unit: "",
            options: ParamOptions::DEFAULT,
        }
        .integer(),
        ParamSpec {
            name: "offset",
            label: "Offset",
            description: "Fires this many triggers earlier in the count.",
            min: 0.0,
            max: 63.0,
            default: 0.0,
            unit: "",
            options: ParamOptions::DEFAULT,
        }
        .integer(),
    ],
    role: ModuleRole::Dsp(|_| Box::new(Divide::new())),
    panel: Panel::new(6, 3, &[Widget::knob(0, 0, "n"), Widget::knob(3, 0, "offset")]),
    displays: &[],
};

struct Divide {
    n: u64,
    offset: u64,
    /// Triggers counted since the last reset. Kept whole, not modulo N, so
    /// changing N gives the same result as if N had been set at the reset.
    count: u64,
    trigger_in: EdgeDetector,
    reset_in: EdgeDetector,
}

impl Divide {
    fn new() -> Self {
        Self {
            n: 2,
            offset: 0,
            count: 0,
            trigger_in: EdgeDetector::new(),
            reset_in: EdgeDetector::new(),
        }
    }

    /// Counts one trigger. True when this trigger fires the output.
    fn count_trigger(&mut self, n: u64, offset: u64) -> bool {
        self.count = self.count.wrapping_add(1);
        let offset = offset.min(n - 1);
        (self.count + offset).is_multiple_of(n)
    }
}

impl Module for Divide {
    fn process(&mut self, _ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let n = io.modulated(0);
        let offset = io.modulated(1);
        let ([trigger_in, reset], [out]) = io.split::<2, 1>();
        for (i, ((out, &t), &r)) in out.iter_mut().zip(trigger_in).zip(reset).enumerate() {
            // Modulated values live only for this frame. The fields keep the
            // knob's base, so removing an assignment returns to it.
            let n = n.map_or(self.n, |samples| (samples[i] as u64).max(1));
            let offset = offset.map_or(self.offset, |samples| samples[i] as u64);
            // Reset first, so a trigger on the same frame as a reset counts
            // as the first trigger after it.
            if self.reset_in.rising(r) {
                self.count = 0;
            }
            let fired = self.trigger_in.rising(t) && self.count_trigger(n, offset);
            *out = trigger(fired);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        // The engine clamps and rounds whole-number parameters, so the
        // casts below are exact. `max(1)` guards the division all the same.
        match index {
            0 => self.n = (value as u64).max(1),
            1 => self.offset = value as u64,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.count = 0;
        self.trigger_in.reset();
        self.reset_in.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loom_core::testing::run;

    /// One frame per step: a trigger frame, then a low frame.
    fn pulses(count: usize) -> Vec<f32> {
        (0..count * 2).map(|i| if i % 2 == 0 { 1.0 } else { 0.0 }).collect()
    }

    /// The numbers (from 1) of the input triggers that fired the output.
    fn fired(n: f32, offset: f32, trigger_in: &[f32], reset: &[f32]) -> Vec<usize> {
        let mut module = Divide::new();
        module.set_param(0, DIVIDE.params[0].clamp(n));
        module.set_param(1, DIVIDE.params[1].clamp(offset));
        let mut out = vec![0.0; trigger_in.len()];
        run(
            &mut module,
            48_000.0,
            &[trigger_in, reset],
            &[true, true],
            &mut [&mut out],
            &[],
        );
        let mut number = 0;
        let mut result = Vec::new();
        for (frame, &t) in trigger_in.iter().enumerate() {
            if t > 0.5 && (frame == 0 || trigger_in[frame - 1] <= 0.5) {
                number += 1;
            }
            if out[frame] == 1.0 {
                result.push(number);
            } else {
                assert_eq!(out[frame], 0.0);
            }
        }
        result
    }

    #[test]
    fn n_2_offset_0_fires_on_2_4_6() {
        let input = pulses(6);
        assert_eq!(fired(2.0, 0.0, &input, &vec![0.0; input.len()]), [2, 4, 6]);
    }

    #[test]
    fn n_2_offset_1_fires_on_1_3_5() {
        let input = pulses(6);
        assert_eq!(fired(2.0, 1.0, &input, &vec![0.0; input.len()]), [1, 3, 5]);
    }

    #[test]
    fn offset_n_minus_1_fires_on_the_first_trigger() {
        let input = pulses(9);
        assert_eq!(fired(4.0, 3.0, &input, &vec![0.0; input.len()]), [1, 5, 9]);
    }

    #[test]
    fn n_1_fires_on_every_trigger() {
        let input = pulses(4);
        assert_eq!(fired(1.0, 0.0, &input, &vec![0.0; input.len()]), [1, 2, 3, 4]);
    }

    #[test]
    fn offset_of_n_or_more_counts_as_n_minus_1() {
        let input = pulses(7);
        let silent = vec![0.0; input.len()];
        assert_eq!(fired(3.0, 10.0, &input, &silent), fired(3.0, 2.0, &input, &silent));
        assert_eq!(fired(3.0, 10.0, &input, &silent), [1, 4, 7]);
    }

    #[test]
    fn reset_starts_the_count_again() {
        // Triggers 1 to 3, a reset, then triggers 4 to 7.
        let input = pulses(7);
        let mut reset = vec![0.0; input.len()];
        reset[5] = 1.0;
        // Without the reset: 2, 4, 6. After it, counting starts again at
        // trigger 4, so the second and fourth after it fire: 5 and 7.
        assert_eq!(fired(2.0, 0.0, &input, &reset), [2, 5, 7]);
    }

    #[test]
    fn trigger_on_the_reset_frame_counts_as_the_first() {
        let input = pulses(5);
        let mut reset = vec![0.0; input.len()];
        // Reset on the same frame as trigger 2.
        reset[2] = 1.0;
        assert_eq!(fired(2.0, 0.0, &input, &reset), [3, 5]);
    }

    #[test]
    fn a_held_gate_counts_once() {
        let mut input = vec![1.0; 10];
        input.extend([0.0, 1.0, 1.0, 0.0]);
        assert_eq!(fired(2.0, 0.0, &input, &vec![0.0; input.len()]), [2]);
    }

    #[test]
    fn changing_n_keeps_the_count_since_reset() {
        // Five triggers at N = 2, then three more at N = 3.
        let mut module = Divide::new();
        let first = pulses(5);
        let mut out = vec![0.0; first.len()];
        let silent = vec![0.0; first.len()];
        run(
            &mut module,
            48_000.0,
            &[&first, &silent],
            &[true, true],
            &mut [&mut out],
            &[],
        );
        assert_eq!(out.iter().filter(|&&v| v == 1.0).count(), 2, "triggers 2 and 4");

        module.set_param(0, 3.0);
        let second = pulses(3);
        let mut out = vec![0.0; second.len()];
        let silent = vec![0.0; second.len()];
        run(
            &mut module,
            48_000.0,
            &[&second, &silent],
            &[true, true],
            &mut [&mut out],
            &[],
        );
        // The count goes on from 5, so only trigger 6 fires, as if N had been
        // 3 since the reset. A count kept modulo 2 would be at 1, and fire
        // on trigger 7 instead.
        let fired: Vec<usize> = (0..out.len()).filter(|&i| out[i] == 1.0).map(|i| i / 2 + 6).collect();
        assert_eq!(fired, [6]);
    }

    #[test]
    fn same_output_for_any_block_size() {
        let input: Vec<f32> = (0..500).map(|i| if i % 7 < 2 { 1.0 } else { 0.0 }).collect();
        let reset: Vec<f32> = (0..500).map(|i| if i % 97 == 50 { 1.0 } else { 0.0 }).collect();
        let render = |block: usize| {
            let mut module = Divide::new();
            module.set_param(0, 3.0);
            module.set_param(1, 1.0);
            let mut out = vec![0.0; input.len()];
            for ((out, t), r) in out.chunks_mut(block).zip(input.chunks(block)).zip(reset.chunks(block)) {
                run(&mut module, 48_000.0, &[t, r], &[true, true], &mut [out], &[]);
            }
            out
        };
        let whole = render(500);
        assert!(whole.contains(&1.0));
        for block in [1, 3, 64] {
            assert_eq!(render(block), whole, "block size {block}");
        }
    }
}
