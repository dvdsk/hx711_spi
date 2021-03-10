use std::error::Error;
use rppal::spi::{Bus, Mode, SlaveSelect, Spi, Segment};

// constants from rppal documentation / example
const WRITE: u8 = 0b0010; // Write data, starting at the selected address.
const READ: u8 = 0b0011; // Read data, starting at the selected address.
const RDSR: u8 = 0b0101; // Read the STATUS register.
const WREN: u8 = 0b0110; // Set the write enable latch (enable write operations).

const WIP: u8 = 1; // Write-In-Process bit mask for the STATUS register.
/// The HX711 can run in three modes:
pub enum HX711Mode {
    /// Chanel A with factor 128 gain
    ChAGain128 = 0x80,
    /// Chanel B with factor 64 gain
    ChBGain32 = 0xC0,
    /// Chanel B with factor 32 gain
    ChBGain64 = 0xE0,
}

pub struct Hx711
{
    spi: Spi,
    mode: HX711Mode
}

impl Hx711
{
    pub fn new(/*bus: Bus*/) -> Result<Hx711, Box<dyn Error>>
    {
        let dev = Spi::new(Bus::Spi0, SlaveSelect::Ss0, 1_000_000, Mode::Mode0)?;

        Ok
        (
            Hx711
            {
                spi: dev,
                mode: HX711Mode::ChAGain128
            }
        )
    }

    pub fn readout(&self, nr_values: u8) -> Result<i32, Box<dyn Error>>
    {
        // "write" transfers are also reads at the same time with
        // the read having the same length as the write
        let tx_buf = [0xaa, 0xaa, 0xaa, self.mode as u8];
        let mut rx_buf = [0; 4];
        let mut values: Vec<i32> = Vec::new();
        let mut result: i32 = 0;

        for _i in 1..=nr_values
        {
            self.spi.write(&[WREN])?;                        // write enable

            let transfer = Segment::new(&mut rx_buf, &tx_buf);
            self.spi.transfer_segments(&[transfer])?;
            println!("{:?}", rx_buf);
            values.push(i32::from_be_bytes(rx_buf));
        }

        // arithmetic average over the values
        for element in values.iter()
        {
            result = result + element;
        }
        result = result / nr_values as i32;
        Ok(result / 0x100)                                 // return value (upper 24 bits)
    }
}

fn main() {
    println!("Hello, world!");
}
