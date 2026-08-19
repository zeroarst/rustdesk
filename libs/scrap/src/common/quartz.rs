use crate::{common::frame_slot::FrameSlot, quartz, Frame, Pixfmt};
use std::marker::PhantomData;
use std::sync::Arc;
use std::{io, time::Duration};

pub struct Capturer {
    inner: quartz::Capturer,
    slot: Arc<FrameSlot<quartz::Frame>>,
}

impl Capturer {
    pub fn new(display: Display) -> io::Result<Capturer> {
        let slot = Arc::new(FrameSlot::new());

        let producer = slot.clone();
        let inner = quartz::Capturer::new(
            display.0,
            display.width(),
            display.height(),
            quartz::PixelFormat::Argb8888,
            Default::default(),
            move |frame| {
                // Store the newest frame, replacing any the consumer has not
                // collected yet, and wake a waiting `frame()`.
                producer.put(frame);
            },
        )
        .map_err(|_| io::Error::from(io::ErrorKind::Other))?;

        Ok(Capturer { inner, slot })
    }

    pub fn width(&self) -> usize {
        self.inner.width()
    }

    pub fn height(&self) -> usize {
        self.inner.height()
    }
}

impl crate::TraitCapturer for Capturer {
    fn frame<'a>(&'a mut self, timeout: Duration) -> io::Result<Frame<'a>> {
        // Wait (up to `timeout`) for the capture callback to deliver a frame,
        // instead of polling. The IOSurface behind the frame is read-locked
        // for the frame's lifetime, so its pixels are handed downstream
        // without a copy.
        match self.slot.take(timeout) {
            Some(frame) => Ok(Frame::PixelBuffer(PixelBuffer {
                frame,
                data: PhantomData,
                width: self.width(),
                height: self.height(),
            })),
            None => Err(io::ErrorKind::WouldBlock.into()),
        }
    }
}

pub struct PixelBuffer<'a> {
    frame: quartz::Frame,
    data: PhantomData<&'a [u8]>,
    width: usize,
    height: usize,
}

impl<'a> crate::TraitPixelBuffer for PixelBuffer<'a> {
    fn data(&self) -> &[u8] {
        &*self.frame
    }

    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }

    fn stride(&self) -> Vec<usize> {
        let mut v = Vec::new();
        v.push(self.frame.stride());
        v
    }

    fn pixfmt(&self) -> Pixfmt {
        Pixfmt::BGRA
    }
}

pub struct Display(quartz::Display);

impl Display {
    pub fn primary() -> io::Result<Display> {
        Ok(Display(quartz::Display::primary()))
    }

    pub fn all() -> io::Result<Vec<Display>> {
        Ok(quartz::Display::online()
            .map_err(|_| io::Error::from(io::ErrorKind::Other))?
            .into_iter()
            .map(Display)
            .collect())
    }

    pub fn width(&self) -> usize {
        self.0.width()
    }

    pub fn height(&self) -> usize {
        self.0.height()
    }

    pub fn scale(&self) -> f64 {
        self.0.scale()
    }

    pub fn name(&self) -> String {
        self.0.id().to_string()
    }

    pub fn is_online(&self) -> bool {
        self.0.is_online()
    }

    pub fn origin(&self) -> (i32, i32) {
        let o = self.0.bounds().origin;
        (o.x as _, o.y as _)
    }

    pub fn is_primary(&self) -> bool {
        self.0.is_primary()
    }
}
