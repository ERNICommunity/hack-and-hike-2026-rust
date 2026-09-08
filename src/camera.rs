//! CPU0-owned CoreS3 Lite camera capture.
//!
//! The onboard GC0308 emits native QVGA RGB565 over its 8-bit DVP bus. LCD_CAM
//! receives complete 320x240 frames into one permanently allocated, cache-line
//! aligned PSRAM buffer. The UI scales the complete frame to fit without cropping.

mod gc0308;
mod sccb;

use esp_hal::{
    delay::Delay,
    dma::{DmaRxBuf, ExternalBurstConfig},
    lcd_cam::{LcdCam, cam::{Camera as CameraDriver, Config as CameraConfig}},
    peripherals::{
        DMA_CH2, GPIO11, GPIO12, GPIO15, GPIO16, GPIO38, GPIO39, GPIO40, GPIO41, GPIO42,
        GPIO45, GPIO46, GPIO47, GPIO48, LCD_CAM,
    },
    time::Rate,
};

use crate::data_plane;

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 240;
const BYTES_PER_PIXEL: usize = 2;
const FRAME_BYTES: usize = WIDTH * HEIGHT * BYTES_PER_PIXEL;
const PSRAM_ALIGNMENT: usize = 32;
const DMA_CHUNK_BYTES: usize = 4064;

#[repr(C, align(32))]
#[derive(Clone, Copy)]
struct AlignedBlock([u8; PSRAM_ALIGNMENT]);

pub struct Resources {
    pub lcd_cam: LCD_CAM<'static>,
    pub dma: DMA_CH2<'static>,
    pub pclk: GPIO45<'static>,
    pub vsync: GPIO46<'static>,
    pub href: GPIO38<'static>,
    pub d0: GPIO39<'static>,
    pub d1: GPIO40<'static>,
    pub d2: GPIO41<'static>,
    pub d3: GPIO42<'static>,
    pub d4: GPIO15<'static>,
    pub d5: GPIO16<'static>,
    pub d6: GPIO48<'static>,
    pub d7: GPIO47<'static>,
}

pub struct Camera {
    driver: Option<CameraDriver<'static>>,
    buffer: Option<DmaRxBuf>,
}

pub struct Frame<'a> {
    bytes: &'a [u8],
}

pub struct SensorInit {
    pub pid: u8,
    pub nack_count: u16,
}

impl Frame<'_> {
    pub fn scanline(&self, y: usize) -> &[u8] {
        debug_assert!(y < HEIGHT);
        let start = y * WIDTH * BYTES_PER_PIXEL;
        &self.bytes[start..start + WIDTH * BYTES_PER_PIXEL]
    }
}

/// Program the GC0308 over a startup-only software SCCB owner.
///
/// GPIO12/GPIO11 are shared with the board's normal system I2C bus. Bootstrap
/// deliberately drops its temporary hardware-I2C driver before calling this
/// function, matching M5Stack's camera ownership model. After this function
/// returns, the software SCCB pins are dropped and the persistent 400 kHz
/// system-I2C driver can be created for CPU1.
pub fn init_sensor(sda: GPIO12<'_>, scl: GPIO11<'_>, delay: &mut Delay) -> SensorInit {
    let mut sccb = sccb::Sccb::new(sda, scl);
    sccb.recover_bus();

    let pid = gc0308::init(&mut sccb, delay).unwrap_or_else(|never| match never {});
    SensorInit {
        pid,
        nack_count: sccb.nack_count(),
    }
}

pub const EXPECTED_SENSOR_PID: u8 = gc0308::EXPECTED_PID;

pub fn init(resources: Resources) -> Camera {
    let Resources {
        lcd_cam,
        dma,
        pclk,
        vsync,
        href,
        d0,
        d1,
        d2,
        d3,
        d4,
        d5,
        d6,
        d7,
    } = resources;

    let lcd_cam = LcdCam::new(lcd_cam);
    let config = CameraConfig::default().with_frequency(Rate::from_mhz(20));
    let driver = CameraDriver::new(lcd_cam.cam, dma, config)
        .expect("Failed to configure LCD_CAM camera input")
        // CoreS3 Lite has no MCU-driven camera XCLK pin, so this deliberately
        // stays in LCD_CAM slave mode.
        .with_pixel_clock(pclk)
        .with_vsync(vsync)
        .with_h_enable(href)
        .with_data0(d0)
        .with_data1(d1)
        .with_data2(d2)
        .with_data3(d3)
        .with_data4(d4)
        .with_data5(d5)
        .with_data6(d6)
        .with_data7(d7);

    let blocks = data_plane::leaked_filled_slice(
        FRAME_BYTES / PSRAM_ALIGNMENT,
        AlignedBlock([0; PSRAM_ALIGNMENT]),
    );
    // AlignedBlock gives this allocation exclusive ownership of whole 32-byte
    // PSRAM cache lines for its entire device lifetime.
    let frame_bytes = unsafe {
        core::slice::from_raw_parts_mut(blocks.as_mut_ptr().cast::<u8>(), FRAME_BYTES)
    };
    let (rx_descriptors, _tx_descriptors) =
        esp_hal::dma_descriptors_chunk_size!(FRAME_BYTES, DMA_CHUNK_BYTES);
    let buffer = DmaRxBuf::new_with_config(
        rx_descriptors,
        frame_bytes,
        ExternalBurstConfig::Size32,
    )
    .expect("Failed to construct camera DMA buffer");

    Camera {
        driver: Some(driver),
        buffer: Some(buffer),
    }
}

impl Camera {
    /// Capture one complete QVGA frame. Failed frames are dropped while keeping
    /// the camera and its fixed DMA storage ready for the next attempt.
    pub fn capture(&mut self) -> Option<Frame<'_>> {
        let driver = self.driver.take().expect("Camera driver missing");
        let buffer = self.buffer.take().expect("Camera DMA buffer missing");

        let transfer = match driver.receive(buffer) {
            Ok(transfer) => transfer,
            Err((error, driver, buffer)) => {
                log::warn!("Camera DMA start failed: {:?}", error);
                self.driver = Some(driver);
                self.buffer = Some(buffer);
                return None;
            }
        };

        let (result, driver, buffer) = transfer.wait();
        let received = buffer.number_of_received_bytes();
        let complete = result.is_ok() && received >= FRAME_BYTES;
        if let Err(error) = result {
            log::warn!("Camera frame DMA failed: {:?}", error);
        } else if received < FRAME_BYTES {
            log::warn!("Camera short frame: {} / {} bytes", received, FRAME_BYTES);
        }

        self.driver = Some(driver);
        self.buffer = Some(buffer);
        if !complete {
            return None;
        }

        let bytes = &self.buffer.as_ref()?.as_slice()[..FRAME_BYTES];
        Some(Frame { bytes })
    }
}
