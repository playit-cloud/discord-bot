use std::{fmt::{Debug, Display}, panic::Location};

pub trait LogHelpers: Sized {
    fn log_error<M: Display>(self, message: M) -> Self;

    fn log_warn<M: Display>(self, message: M) -> Self;
}

impl<S, E: Debug> LogHelpers for Result<S, E> {
    #[track_caller]
    fn log_error<M: Display>(self, message: M) -> Self {
        match self {
            Ok(v) => Ok(v),
            Err(error) => {
                tracing::error!(?error, caller = %Location::caller(), "{}", message);
                Err(error)
            }
        }
    }

    #[track_caller]
    fn log_warn<M: Display>(self, message: M) -> Self {
        match self {
            Ok(v) => Ok(v),
            Err(error) => {
                tracing::warn!(?error, caller = %Location::caller(), "{}", message);
                Err(error)
            }
        }
    }
}

impl<S> LogHelpers for Option<S> {
    #[track_caller]
    fn log_error<M: Display>(self, message: M) -> Self {
        match self {
            Some(v) => Some(v),
            None => {
                tracing::error!(caller = %Location::caller(), "{}", message);
                None
            }
        }
    }

    #[track_caller]
    fn log_warn<M: Display>(self, message: M) -> Self {
        match self {
            Some(v) => Some(v),
            None => {
                tracing::warn!(caller = %Location::caller(), "{}", message);
                None
            }
        }
    }
}

