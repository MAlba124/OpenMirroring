pub mod loading;
pub mod primary;
pub mod select_source;
pub mod signaller;

pub type ProducerId = String;

#[derive(Debug)]
pub enum Event {
    ProducerConnected(ProducerId),
    Start,
    Stop,
    EnablePreview,
    DisablePreview,
    Sources(Vec<String>),
    SelectSource(usize),
}
