//! The order a job runs its passes in. A pass that is stopped ends the round, and leaves its unfinished work where the
//! next round finds it (the passes take their work from the job's queue as they go), so the round itself keeps
//! nothing.

use crate::failure::Failed;

/// How a pass ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// The pass did all it had for this round.
    Done,
    /// The pass was stopped before it finished: the round stops too.
    Preempted,
}

/// One pass of a job, over the job's context `C`.
pub trait Pass<C> {
    /// Runs the pass over `cx`, and says whether it is done or was preempted (and must run again).
    fn run(&mut self, cx: &mut C) -> Result<Step, Failed>;
}

/// Run the passes in the given order. A pass that is preempted ends the round at once, and so does a stop requested
/// while a pass ran (`stopped`), so the passes after it don't start. A pass that fails does not end the round: its error is
/// kept, the passes after it still run, and the round returns the first error at its end (or at the preempt that ends it).
pub fn round<C>(passes: &mut [&mut dyn Pass<C>], cx: &mut C, stopped: impl Fn(&C) -> bool) -> Result<Step, Failed> {
    let mut first: Option<Failed> = None;
    for pass in passes.iter_mut() {
        let preempted = match pass.run(cx) {
            Ok(step) => step == Step::Preempted,
            Err(e) => {
                first.get_or_insert(e);
                false
            }
        };
        if preempted || stopped(cx) {
            return first.map_or(Ok(Step::Preempted), Err);
        }
    }
    first.map_or(Ok(Step::Done), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records its name in the context, and answers with the step it is given.
    struct Named(&'static str, Step);

    impl Pass<Vec<&'static str>> for Named {
        fn run(&mut self, cx: &mut Vec<&'static str>) -> Result<Step, Failed> {
            cx.push(self.0);
            Ok(match self.1 {
                Step::Done => Step::Done,
                Step::Preempted => Step::Preempted,
            })
        }
    }

    /// Records its name in the context, and fails with it.
    struct Fails(&'static str);

    impl Pass<Vec<&'static str>> for Fails {
        fn run(&mut self, cx: &mut Vec<&'static str>) -> Result<Step, Failed> {
            cx.push(self.0);
            Err(Failed::other(self.0.to_string()))
        }
    }

    #[test]
    fn the_passes_run_in_order_when_none_is_preempted() {
        let (mut a, mut b) = (Named("a", Step::Done), Named("b", Step::Done));
        let mut cx = Vec::new();
        let step = round(&mut [&mut a, &mut b], &mut cx, |_| false).unwrap();
        assert_eq!((step, cx), (Step::Done, vec!["a", "b"]));
    }

    #[test]
    fn a_preempted_pass_ends_the_round_and_the_later_ones_do_not_run() {
        let (mut a, mut b) = (Named("a", Step::Preempted), Named("b", Step::Done));
        let mut cx = Vec::new();
        let step = round(&mut [&mut a, &mut b], &mut cx, |_| false).unwrap();
        assert_eq!((step, cx), (Step::Preempted, vec!["a"]));
    }

    #[test]
    fn a_stop_requested_during_a_pass_ends_the_round_after_it() {
        let (mut a, mut b) = (Named("a", Step::Done), Named("b", Step::Done));
        let mut cx = Vec::new();
        let step = round(&mut [&mut a, &mut b], &mut cx, |cx| cx.contains(&"a")).unwrap();
        assert_eq!((step, cx), (Step::Preempted, vec!["a"]));
    }

    /// A failed pass does not stop the round: the passes after it run, and the round returns the first error at its end.
    #[test]
    fn an_error_is_kept_the_later_passes_run_and_the_round_returns_the_first() {
        let (mut f, mut a, mut g) = (Fails("f"), Named("a", Step::Done), Fails("g"));
        let mut cx = Vec::new();
        let error = round(&mut [&mut f, &mut a, &mut g], &mut cx, |_| false).unwrap_err();
        assert_eq!((error.text(), cx), ("f", vec!["f", "a", "g"]));
    }

    /// A preempt after a failed pass ends the round, and the round still returns the failure.
    #[test]
    fn a_preempt_after_an_error_ends_the_round_with_the_error() {
        let (mut f, mut a, mut b) = (Fails("f"), Named("a", Step::Preempted), Named("b", Step::Done));
        let mut cx = Vec::new();
        let error = round(&mut [&mut f, &mut a, &mut b], &mut cx, |_| false).unwrap_err();
        assert_eq!((error.text(), cx), ("f", vec!["f", "a"]));
    }
}
