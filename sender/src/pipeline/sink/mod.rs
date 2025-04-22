use gst::glib;

pub mod hls;
pub mod webrtc;

#[async_trait::async_trait]
pub trait TransmissionSink: Send {
    /// Get the message that should be sent to a receiver to consume the stream
    fn get_play_msg(&self) -> Option<crate::Message>;

    /// Called when the pipeline enters the playing state
    async fn playing(&mut self);

    /// Perform any necessary shutdown procedures
    fn shutdown(&mut self);

    /// Remove the sink's elements from the pipeline and unlink them from the source
    fn unlink(&mut self, pipeline: &gst::Pipeline) -> Result<(), glib::error::BoolError>;
}
