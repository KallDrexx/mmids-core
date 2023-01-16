//! This module provides abstractions over MPSC channels, which make it easy for workflow steps
//! to execute a future and send the results of those futures back to the correct workflow runner
//! with minimal allocations.

use crate::workflows::definitions::WorkflowStepId;
use crate::workflows::steps::StepFutureResult;
use std::future::Future;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;
use crate::workflows::MediaNotification;

/// An channel which can be used by workflow steps to send future completion results to the
/// workflow runner.
#[derive(Clone)]
pub struct WorkflowStepFuturesChannel {
    step_id: WorkflowStepId,
    step_future_result_sender: UnboundedSender<StepFutureResultChannel>,
    media_result_sender: UnboundedSender<FuturesMediaChannelResult>,
}

/// The type of information that's returned to the workflow runner upon a future's completion
pub struct StepFutureResultChannel {
    pub step_id: WorkflowStepId,
    pub result: Box<dyn StepFutureResult>,
}

/// The type of information that's returned to the workflow runner when a future completes with a
/// media result. This is separate from normal workflow step future results, as we will not need
/// to box the media up since it's a defined type.
pub struct FuturesMediaChannelResult {
    pub step_id: WorkflowStepId,
    pub media: MediaNotification,
}

impl WorkflowStepFuturesChannel {
    pub fn new(
        step_id: WorkflowStepId,
        step_future_result_sender: UnboundedSender<StepFutureResultChannel>,
        media_result_sender: UnboundedSender<FuturesMediaChannelResult>,
    ) -> Self {
        WorkflowStepFuturesChannel { step_id, step_future_result_sender, media_result_sender }
    }

    /// Sends the workflow step's future result over the channel. Returns an error if the channel
    /// is closed.
    pub fn send_step_future_result(
        &self,
        message: impl StepFutureResult,
    ) -> Result<(), Box<dyn StepFutureResult>> {
        let message = StepFutureResultChannel {
            step_id: self.step_id,
            result: Box::new(message),
        };

        self.step_future_result_sender.send(message).map_err(|e| e.0.result)
    }

    /// Completes when the channel is closed due to there being no receiver.
    pub async fn closed(&self) {
        // It's not valid for only one of these channels to be open, so consider the channel closed
        // when at least one channel is closed.
        tokio::select! {
            _ = self.step_future_result_sender.closed() => (),
            _ = self.media_result_sender.closed() => (),
        }
    }

    /// Helper function for workflow steps to watch a receiver for messages, and send them back
    /// to the workflow step for processing.
    pub fn send_on_unbounded_recv<ReceiverMessage, FutureResult>(
        &self,
        mut receiver: UnboundedReceiver<ReceiverMessage>,
        on_recv: impl Fn(ReceiverMessage) -> FutureResult + Send + 'static,
        on_closed: impl FnOnce() -> FutureResult + Send + 'static,
    ) where
        ReceiverMessage: Send + 'static,
        FutureResult: StepFutureResult + Send + 'static,
    {
        let channel = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    message = receiver.recv() => {
                        match message {
                            Some(message) => {
                                let future_result = on_recv(message);
                                let _ = channel.send_step_future_result(future_result);
                            }

                            None => {
                                let future_result = on_closed();
                                let _ = channel.send_step_future_result(future_result);
                                break;
                            }
                        }
                    }

                    _ = channel.closed() => {
                        break;
                    }
                }
            }
        });
    }

    /// Helper function for workflow steps to watch a receiver for messages, and send them back
    /// to the workflow step for processing. Cancellable via a token.
    pub fn send_on_unbounded_recv_cancellable<ReceiverMessage, FutureResult>(
        &self,
        mut receiver: UnboundedReceiver<ReceiverMessage>,
        cancellation_token: CancellationToken,
        on_recv: impl Fn(ReceiverMessage) -> FutureResult + Send + 'static,
        on_closed: impl FnOnce() -> FutureResult + Send + 'static,
        on_cancelled: impl FnOnce() -> FutureResult + Send + 'static,
    ) where
        ReceiverMessage: Send + 'static,
        FutureResult: StepFutureResult + Send + 'static,
    {
        let channel = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    message = receiver.recv() => {
                        match message {
                            Some(message) => {
                                let future_result = on_recv(message);
                                let _ = channel.send_step_future_result(future_result);
                            }

                            None => {
                                let future_result = on_closed();
                                let _ = channel.send_step_future_result(future_result);
                                break;
                            }
                        }
                    }

                    _ = cancellation_token.cancelled() => {
                        let future_result = on_cancelled();
                        let _ = channel.send_step_future_result(future_result);
                        break;
                    }

                    _ = channel.closed() => {
                        // Nothing ot send since the channel is closed
                        break;
                    }
                }
            }
        });
    }

    /// Helper function for workflow steps to track a tokio watch receiver for messages, and send
    /// them back to the workflow step for processing.
    pub fn send_on_watch_recv<ReceiverMessage, FutureResult>(
        &self,
        mut receiver: tokio::sync::watch::Receiver<ReceiverMessage>,
        on_recv: impl Fn(&ReceiverMessage) -> FutureResult + Send + 'static,
        on_closed: impl FnOnce() -> FutureResult + Send + 'static,
    ) where
        ReceiverMessage: Send + Sync + 'static,
        FutureResult: StepFutureResult + Send + 'static,
    {
        let channel = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    message = receiver.changed() => {
                        match message {
                            Ok(_) => {
                                let value = receiver.borrow();
                                let future_result = on_recv(&value);
                                let _ = channel.send_step_future_result(future_result);
                            }

                            Err(_) => {
                                let future_result = on_closed();
                                let _ = channel.send_step_future_result(future_result);
                                break;
                            }
                        }
                    }

                    _ = channel.closed() => {
                        break;
                    }
                }
            }
        });
    }

    /// Helper function for workflow steps to easily send a message upon future completion
    pub fn send_on_future_completion(
        &self,
        future: impl Future<Output = impl StepFutureResult + Send> + Send + 'static,
    ) {
        let channel = self.clone();
        tokio::spawn(async move {
            tokio::select! {
                result = future => {
                    let _ = channel.send_step_future_result(result);
                }

                _ = channel.closed() => {
                    // No where to send the result, so cancel the future by exiting
                }
            }
        });
    }
}
