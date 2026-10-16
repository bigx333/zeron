//! Discrete native feedback for user-driven control detents.

/// What a control just did, for the trackpad to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Feedback {
    /// Crossed a detent.
    Step,
    /// Crossed into the top of a range (an effort slider's top tiers): the
    /// first knock of a firmer double one; the caller plays [`Feedback::Step`]
    /// a beat later.
    Peak,
    /// Pushed against the end of a range.
    Edge,
}

pub(crate) fn selection_step() {
    perform(Feedback::Step);
}

pub(crate) fn perform(feedback: Feedback) {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        use objc2_app_kit::{
            NSHapticFeedbackManager, NSHapticFeedbackPattern, NSHapticFeedbackPerformanceTime,
            NSHapticFeedbackPerformer,
        };
        let pattern = match feedback {
            Feedback::Step => NSHapticFeedbackPattern::LevelChange,
            Feedback::Peak => NSHapticFeedbackPattern::Generic,
            Feedback::Edge => NSHapticFeedbackPattern::Alignment,
        };
        // UI input handlers run on the main thread. AppKit gracefully does
        // nothing on devices without a haptic-capable trackpad, or when no
        // finger is on it.
        if objc2::MainThreadMarker::new().is_some() {
            NSHapticFeedbackManager::defaultPerformer().performFeedbackPattern_performanceTime(
                pattern,
                NSHapticFeedbackPerformanceTime::Now,
            );
        }
    }
    #[cfg(test)]
    PLAYED.with(|played| played.borrow_mut().push(feedback));
}

#[cfg(test)]
thread_local! {
    static PLAYED: std::cell::RefCell<Vec<Feedback>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Detent steps played on this thread so far.
#[cfg(test)]
pub(crate) fn step_count() -> usize {
    played().iter().filter(|f| **f == Feedback::Step).count()
}

/// Everything played on this thread so far, in order.
#[cfg(test)]
pub(crate) fn played() -> Vec<Feedback> {
    PLAYED.with(|played| played.borrow().clone())
}
