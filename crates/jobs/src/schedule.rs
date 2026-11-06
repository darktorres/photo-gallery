//! The order a job runs its passes in. A pass that is stopped ends the round, and leaves its unfinished work where the
//! next round finds it (the passes take their work from the job's queue as they go), so the round itself keeps
//! nothing.

use anyhow::Result;

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
    fn run(&mut self, cx: &mut C) -> Result<Step>;
}

/// Run the passes in the given order. A pass that is preempted ends the round at once, and so does a stop requested
/// while a pass ran (`stopped`), so the passes after it don't start.
pub fn round<C>(passes: &mut [&mut dyn Pass<C>], cx: &mut C, stopped: impl Fn(&C) -> bool) -> Result<Step> {
    for pass in passes.iter_mut() {
        if pass.run(cx)? == Step::Preempted || stopped(cx) {
            return Ok(Step::Preempted);
        }
    }
    Ok(Step::Done)
}

#[cfg(test)]
mod tests {
    use anyhow::bail;

    use super::*;

    /// Records its name in the context, and answers with the step it is given.
    struct Named(&'static str, Step);

    impl Pass<Vec<&'static str>> for Named {
        fn run(&mut self, cx: &mut Vec<&'static str>) -> Result<Step> {
            cx.push(self.0);
            Ok(match self.1 {
                Step::Done => Step::Done,
                Step::Preempted => Step::Preempted,
            })
        }
    }

    struct Fails;

    impl Pass<Vec<&'static str>> for Fails {
        fn run(&mut self, _: &mut Vec<&'static str>) -> Result<Step> {
            bail!("no")
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

    #[test]
    fn an_error_ends_the_round_with_it() {
        let (mut a, mut f) = (Named("a", Step::Done), Fails);
        let mut cx = Vec::new();
        assert!(round(&mut [&mut a, &mut f], &mut cx, |_| false).is_err());
    }
}
