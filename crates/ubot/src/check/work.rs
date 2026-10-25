use crate::error::UbotError;

/// Step budget of one check run; adversarial input ends in `Limit` instead of a long run.
const MAX_WORK: u64 = 100_000_000;

pub(super) struct Work {
    left: u64,
}

impl Work {
    pub fn new() -> Self {
        Work { left: MAX_WORK }
    }

    pub fn remaining(&self) -> usize {
        usize::try_from(self.left).unwrap_or(usize::MAX)
    }

    pub fn spend(&mut self, n: usize) -> Result<(), UbotError> {
        let n = u64::try_from(n).unwrap_or(u64::MAX);
        match self.left.checked_sub(n) {
            Some(left) => {
                self.left = left;
                Ok(())
            }
            None => {
                self.left = 0;
                Err(UbotError::Limit("the check needs too much work, split the drawing or reduce the input".into()))
            }
        }
    }
}
