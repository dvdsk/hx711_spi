#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![no_std]

use bitmatch::bitmatch;
use core::unimplemented;
use embedded_hal_async::delay;
use embedded_hal_async::spi;

// Bit pattern definitions for the communication with the hx711. All have to be bitwise negate
// for the ```invert-sdo``` feature

// patterns for mode
#[cfg(not(feature = "invert-sdo"))]
const GAIN128: u8 = 0b1000_0000;
#[cfg(feature = "invert-sdo")]
const GAIN128: u8 = 0b0111_1111;

#[cfg(not(feature = "invert-sdo"))]
const GAIN32: u8 = 0b1010_0000;
#[cfg(feature = "invert-sdo")]
const GAIN32: u8 = 0b0101_1111;

#[cfg(not(feature = "invert-sdo"))]
const GAIN64: u8 = 0b1010_1000;
#[cfg(feature = "invert-sdo")]
const GAIN64: u8 = 0b0101_0111;

// SDO provides clock to the HX711's shift register (binary 1010...)
// one clock cycle is '10'. The buffer needs to be double the size of the 4 bytes we want to read
#[cfg(not(feature = "invert-sdo"))]
const CLOCK: u8 = 0b10101010;
#[cfg(feature = "invert-sdo")]
const CLOCK: u8 = 0b01010101;

// Signal to send to the HX711 when checking for data ready to be read.
#[cfg(not(feature = "invert-sdo"))]
const SIGNAL_LOW: u8 = 0x0;
#[cfg(feature = "invert-sdo")]
const SIGNAL_LOW: u8 = 0xFF;

// reset signal
#[cfg(not(feature = "invert-sdo"))]
const RESET_SIGNAL: [u8; 301] = [0xFF; 301];
#[cfg(feature = "invert-sdo")]
const RESET_SIGNAL: [u8; 301] = [0x00; 301];

// End bit pattern definitions

/// The HX711 has two channels: `A` for the load cell and `B` for AD conversion of other signals.
/// Channel `A` supports gains of 128 (default) and 64, `B` has a fixed gain of 32.
#[derive(Copy, Clone, defmt::Format)]
#[repr(u8)]
pub enum Mode {
    // bits have to be converted for correct transfer 1 -> 10, 0 -> 00
    /// Convert channel A with a gain factor of 128
    ChAGain128 = GAIN128,
    /// Convert channel B with a gain factor of 32
    ChBGain32 = GAIN32,
    /// Convert channel A with a gain factor of 64
    ChAGain64 = GAIN64, // there is a typo in the official datasheet: in Fig.2 it says channel B instead of A
}

#[derive(defmt::Format)]
pub enum Error<E: defmt::Format> {
    Spi(E),
    /// Device took to long to report ready
    NotReadyInTime,
}

impl<E: defmt::Format> From<E> for Error<E> {
    fn from(error: E) -> Self {
        Self::Spi(error)
    }
}

/// Represents an instance of a HX711 device
#[derive(defmt::Format)]
pub struct Hx711<SPI, DELAY> {
    spi: SPI,
    delay: DELAY,
    // device specific
    mode: Mode,
}
// //  needed to satisfy the trait bound in scales
// impl<SPI> Read<i32, nb::Error<E>> for Hx711<SPI>
// where
// SPI: spi::Transfer<u8, Error = E>
// {
//     fn read(&mut self) -> nb::Result<i32, SPI::Error> {
//         self.read_val()
//     }
// }

impl<SPI, DELAY> Hx711<SPI, DELAY>
where
    DELAY: delay::DelayNs,
    SPI: spi::SpiBus,
    SPI::Error: defmt::Format,
{
    /// opens a connection to a HX711 on a specified SPI.
    ///
    /// The datasheet specifies PD_SCK high time and PD_SCK low time to be in the 0.2 to 50 us range,
    /// therefore bus speed has to be between 5 MHz and 20 kHz. 1 MHz seems to be a good choice.
    /// D is an embedded_hal implementation of DelayMs.
    pub fn new(spi: SPI, delay: DELAY) -> Self {
        Hx711 {
            spi,
            delay,
            mode: Mode::ChAGain128,
        }
    }

    /// reads a value from the HX711 and returns it
    /// # Errors
    /// Returns SPI errors and nb::Error::WouldBlock if data isn't ready to be read from hx711
    pub async fn read_val(&mut self) -> Result<i32, Error<SPI::Error>> {
        // check if data is ready
        // When output data is not ready for retrieval, digital output pin DOUT is high.
        // Serial clock input PD_SCK should be low. When DOUT goes
        // to low, it indicates data is ready for retrieval.
        let mut txrx: [u8; 1] = [SIGNAL_LOW];

        self.spi.transfer_in_place(&mut txrx).await?;

        let mut attempt = 0;
        loop {
            if txrx[0] & 0b01 != 0b01 {
                break;
            }

            // as long as the lowest bit is high there is no data waiting
            if attempt > 1000 {
                return Err(Error::NotReadyInTime);
            }

            attempt += 1;
        }

        let mut buffer: [u8; 7] = [CLOCK, CLOCK, CLOCK, CLOCK, CLOCK, CLOCK, self.mode as u8];

        self.spi.transfer_in_place(&mut buffer).await?;

        Ok(decode_output(&buffer)) // value should be in range 0x800000 - 0x7fffff according to datasheet
    }

    /// Reset the chip to it's default state. Mode is set to convert channel A with a gain factor of 128.
    /// # Errors
    /// Returns SPI errors
    #[inline]
    pub async fn reset(&mut self) -> Result<(), SPI::Error> {
        // when PD_SCK pin changes from low to high and stays at high for longer than 60µs,
        // HX711 enters power down mode.
        // When PD_SCK returns to low, chip will reset and enter normal operation mode.
        // speed is the raw SPI speed -> half bits per second.

        // max SPI clock frequency should be 5 MHz to satisfy the 0.2 us limit for the pulse length
        // we have to output more than 300 bytes to keep the line for at least 60 us high.

        let mut buffer: [u8; 301] = RESET_SIGNAL;

        self.spi.transfer_in_place(&mut buffer).await?;
        self.mode = Mode::ChAGain128; // this is the default mode after reset

        Ok(())
    }

    /// Set the mode to the value specified.
    /// # Errors
    /// Returns SPI errors
    #[inline]
    pub async fn set_mode(&mut self, m: Mode) -> Result<Mode, Error<SPI::Error>> {
        self.mode = m;
        // potentially an issue for the async runtime, might want a loop with an
        // explicit yield or sleep
        self.read_val().await?; // read writes Mode for the next read()
        Ok(m)
    }

    #[inline]
    /// Get the current mode.
    pub fn mode(&mut self) -> Mode {
        self.mode
    }

    #[inline]
    /// This is for compatibility only. Use [mode]() instead.
    pub fn get_mode(&mut self) -> Mode {
        self.mode
    }

    /// To power down the chip the PD_SCK line has to be held in a 'high' state. To do this we
    /// would need to write a constant stream of binary '1' to the SPI bus which would totally defy
    /// the purpose. Therefore it's not implemented.
    // If the SDO pin would be idle high (and at least some MCU's seem to do that in mode 1) then the chip would automatically
    // power down if not used. Cool!
    pub fn disable(&mut self) -> Result<(), SPI::Error> {
        // when PD_SCK pin changes from low to high and stays at high for longer than 60µs, HX711 enters power down mode
        // When PD_SCK returns to low, chip will reset and enter normal operation mode.
        // this can't be implemented with SPI because we would have to write a constant stream
        // of binary '1' which would block the process
        unimplemented!("power_down is not possible with this driver implementation");
    }

    /// Power up / down is not implemented (see disable)
    pub fn enable(&mut self) -> Result<(), SPI::Error> {
        // when PD_SCK pin changes from low to high and stays at high for longer than 60µs, HX711 enters power down mode
        // When PD_SCK returns to low, chip will reset and enter normal operation mode.
        // this can't be implemented with SPI because we would have to write a constant stream
        // of binary '1' which would block the process
        unimplemented!("power_down is not possible with this driver implementation");
    }
}

#[bitmatch]
fn decode_output(buffer: &[u8; 7]) -> i32 {
    // buffer contains the 2's complement of the reading with every bit doubled
    // since the first byte is the most significant it's big endian
    // we have to extract every second bit from the buffer
    // only the upper 24 (doubled) bits are valid

    #[bitmatch]
    let "a?a?a?a?" = buffer[0];
    #[bitmatch]
    let "b?b?b?b?" = buffer[1];
    #[bitmatch]
    let "c?c?c?c?" = buffer[2];
    #[bitmatch]
    let "d?d?d?d?" = buffer[3];
    #[bitmatch]
    let "e?e?e?e?" = buffer[4];
    #[bitmatch]
    let "f?f?f?f?" = buffer[5];

    let mut raw: [u8; 4] = [0; 4];
    raw[0] = bitpack!("aaaabbbb");
    raw[1] = bitpack!("ccccdddd");
    raw[2] = bitpack!("eeeeffff");
    raw[3] = 0;

    i32::from_be_bytes(raw) / 0x100
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;
    // embedded_hal implementation
    use embedded_hal_mock::spi::{Mock as Spi, Transaction as SpiTransaction};

    #[test_case(&[0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55] => 0; "alternating convert to zeros")]
    #[test_case(&[0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA] => -1; "alternating convert to ones")]
    #[test_case(&[0xFF, 0xFF,0xFF,0xFF,0xFF,0xFF,0xFF] => -1; "all ones")]
    #[test_case(&[0b00100111, 0b00100111, 0b00100111, 0b00100111,
                  0b00100111, 0b00100111, 0b00100111] => 0b0000_0000_0101_0101_0101_0101_0101_0101i32; "test pattern")]
    fn test_decode(buffer: &[u8; 7]) -> i32 {
        decode_output(&buffer)
    }

    #[test]
    fn test_read() {
        // Data the mocked up SPI bus should return
        let expectations = [
            SpiTransaction::transfer(vec![SIGNAL_LOW], vec![SIGNAL_LOW]),
            SpiTransaction::transfer(
                vec![CLOCK, CLOCK, CLOCK, CLOCK, CLOCK, CLOCK, GAIN128],
                vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, SIGNAL_LOW],
            ),
        ];

        let spi = Spi::new(&expectations);
        let mut hx711 = Hx711::new(spi);

        //hx711.reset()?;
        let v = block!(hx711.read())?;
        assert_eq!(v, 0);
    }
}
