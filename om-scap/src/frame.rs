// TODO: migrate windows over to this new frame scheme
// TODO: dma buf (https://docs.pipewire.org/page_dma_buf.html)

#[derive(Debug, PartialEq, Eq)]
pub enum FrameFormat {
    RGBx,
    XBGR,
    BGRx,
    BGRA,
    RGBA,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FrameInfo {
    pub format: FrameFormat,
    pub width: u32,
    pub height: u32,
    // TODO: rotation
}
