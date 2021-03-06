#![no_std]
#![no_main]

use panic_halt as _;

use gd32vf103xx_hal::pac;
use gd32vf103xx_hal::prelude::*;
use longan_nano::{lcd, lcd_pins};
use riscv_rt::entry;

use byte_slice_cast::AsSliceOf;
use core::convert::TryInto;
use embedded_graphics::drawable::Pixel;
use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::pixelcolor::raw::RawU16;
use embedded_graphics::prelude::DrawTarget;
use rand::Rng;
use rand_pcg::Pcg32;

// Number of fish on the screen at once.  Does not have to equal NUM_SPRITES.
const NUM_FISH: usize = 10;

// For the two fish that are animated, controls how fast their mouths
// open and close.  Larger is slower.  1 is fastest.
const ANIMATION_SPEED: u8 = 2;

// Color of the water, in RGB565 format.
const BACKGROUND: u16 = 0x1f;   // blue

// This is for making sure that the area around the fish gets erased.
// As long as the fish don't move by more than one pixel at a time,
// 1 should be sufficient.
const FUDGE_FACTOR: i32 = 1;

// These three constants are baked into fish.raw, so don't change them
// unless fish.raw changes.
const NUM_FRAMES: usize = 3;
const NUM_SPRITES: usize = 10;
const TRANSPARENT: u16 = 0xdead;

// This file contains the fish images.
const SPRITE_DATA: &[u8] = include_bytes!("fish.raw");

enum PointValue {
    OutOfRange,
    Transparent,
    Opaque(u16),
}

#[derive(PartialEq, Copy, Clone)]
enum Dir {
    Left,
    Right,
}

#[derive(Copy, Clone)]
struct Sprite<'a> {
    size: Size,
    frames: [&'a [u16]; NUM_FRAMES],
}

#[derive(Copy, Clone)]
struct Fish<'a> {
    fish_type:  Sprite<'a>,
    upper_left: Point,
    size:       Size,
    direction:  Dir,
    animation:  u8,
}

struct FishTank<'a> {
    fish:    [Fish<'a>;   NUM_FISH],
    size:    Size,
    rng:     Pcg32,
}

struct TankIterator<'a> {
    tank:     &'a FishTank<'a>,
    position: Point,
}

fn cvt(u: u32) -> i32 {
    u.try_into().unwrap()
}

fn rgb565(packed: u16) -> Rgb565 {
    Rgb565::from(RawU16::new(packed))
}

impl Sprite<'_> {
    fn get_point(&self, pt: &Point, animation: u8) -> PointValue {
        let x = pt.x - FUDGE_FACTOR;
        let y = pt.y - FUDGE_FACTOR;
        if x < 0 || y < 0 ||
            x >= cvt(self.size.width) ||
            y >= cvt(self.size.height) {
            PointValue::Transparent
        } else {
            let x: usize = x.try_into().unwrap();
            let y: usize = y.try_into().unwrap();
            let width: usize = self.size.width.try_into().unwrap();
            let idx: usize = x + y * width;
            let frame_no: usize = animation.into();
            let frame: &[u16] = self.frames[frame_no];
            let c = frame[idx];
            if c == TRANSPARENT {
                PointValue::Transparent
            } else {
                PointValue::Opaque(c)
            }
        }
    }

    fn make_sprite(sprite_num: usize, sprite_data: &[u16]) -> Sprite {
        let header_index = 4 * sprite_num;
        let width_height = sprite_data[header_index];
        let width = width_height >> 8;
        let height = width_height & 0xff;
        let num_words = width * height;

        let mut sprite = Sprite {
            size: Size::new(width.into(), height.into()),
            frames: [&[]; NUM_FRAMES],
        };

        for frame in 0..3 {
            let frame_index = sprite_data[header_index + frame + 1];
            sprite.frames[frame] =
                &sprite_data[frame_index.into()..(frame_index+num_words).into()];
        }

        sprite
    }
}

impl Fish<'_> {
    fn get_point(&self, pt: &Point) -> PointValue {
        if pt.x < self.upper_left.x ||
            pt.y < self.upper_left.y ||
            pt.x >= self.upper_left.x + cvt(self.size.width) ||
            pt.y >= self.upper_left.y + cvt(self.size.height) {
            PointValue::OutOfRange
        } else {
            let mut x = pt.x - self.upper_left.x;
            let y = pt.y - self.upper_left.y;
            if self.direction == Dir::Left {
                x = cvt(self.size.width) - (x + 1);
            }
            self.fish_type.get_point(&Point::new(x, y),
                                     self.animation / ANIMATION_SPEED)
        }
    }

    fn on_screen(&self, screen: &Size) -> bool {
        self.upper_left.y <= cvt(screen.height) &&
            self.upper_left.y + cvt(self.size.height) >= 0 &&
            self.upper_left.x <= cvt(screen.width) &&
            self.upper_left.x + cvt(self.size.width) >= 0
    }

    fn randomize<T: Rng>(&mut self, screen: &Size, rng: &mut T) {
        let lo: u8 = 0;
        let hi: u8 = NUM_FRAMES.try_into().unwrap();
        self.animation = rng.gen_range(lo, hi * ANIMATION_SPEED);
        if rng.gen() {
            self.direction = Dir::Left;
            self.upper_left.x = cvt(screen.width);
        } else {
            self.direction = Dir::Right;
            self.upper_left.x = -cvt(self.size.width);
        }
        self.upper_left.y =
            cvt(rng.gen_range(0, screen.height - self.size.height));
    }

    fn randomize_x<T: Rng>(&mut self, screen: &Size, rng: &mut T) {
        self.upper_left.x =
            cvt(rng.gen_range(0, screen.width - self.size.width));
    }

    fn swim<T: Rng>(&mut self, screen: &Size, rng: &mut T) {
        if rng.gen_ratio(3, 4) {
            self.upper_left.x += match self.direction {
                Dir::Left => -1,
                Dir::Right => 1,
            }
        }

        if rng.gen_ratio(1, 8) {
            self.upper_left.y += rng.gen_range(-1, 2);
        }

        self.animation += 1;
        let num_frames: u8 = NUM_FRAMES.try_into().unwrap();
        if self.animation >= num_frames * ANIMATION_SPEED {
            self.animation = 0;
        }

        if self.on_screen(screen) == false {
            self.randomize(screen, rng);
        }
    }

    fn new<'a>(sprite: Sprite<'a>) -> Fish<'a> {
        let ff2: u32 = (FUDGE_FACTOR * 2).try_into().unwrap();
        Fish {
            fish_type:  sprite,
            upper_left: Point::new(0, 0),
            size:       Size::new(sprite.size.width + ff2,
                                  sprite.size.height + ff2),
            direction:  Dir::Right,
            animation:  0,
        }
    }
}

impl FishTank<'_> {
    fn new(screen_size: Size, seed: u64) -> FishTank<'static> {
        let sprite_data = SPRITE_DATA.as_slice_of::<u16>().unwrap();
        let dummy_sprite = Sprite::make_sprite(0, sprite_data);
        let mut tank = FishTank {
            fish:    [Fish::new(dummy_sprite); NUM_FISH],
            size:    screen_size,
            rng:     Pcg32::new(seed, 0xdefacedbadfacade),
        };

        for i in 0..NUM_FISH {
            let sprite = Sprite::make_sprite(i % NUM_SPRITES, sprite_data);
            tank.fish[i] = Fish::new(sprite);
            tank.fish[i].randomize  (&tank.size, &mut tank.rng);
            tank.fish[i].randomize_x(&tank.size, &mut tank.rng);
        }

        tank
    }

    fn swim(&mut self) {
        for i in 0..NUM_FISH {
            self.fish[i].swim(&self.size, &mut self.rng);
        }
    }

    fn get_point(&self, pt: &Point) -> PointValue {
        let mut ret = PointValue::OutOfRange;
        for i in 0..NUM_FISH {
            match self.fish[i].get_point(pt) {
                PointValue::Opaque(c)   => return PointValue::Opaque(c),
                PointValue::Transparent => ret = PointValue::Transparent,
                PointValue::OutOfRange  => (),
            }
        }

        ret
    }
}

impl TankIterator<'_> {
    fn new<'a>(fish_tank: &'a FishTank<'a>) -> TankIterator<'a> {
        TankIterator {
            tank:     fish_tank,
            position: Point::new(0, 0),
        }
    }

    fn some_color(&self, c: u16) -> Option<Pixel<Rgb565>> {
        Some(Pixel(self.position, rgb565(c)))
    }
}

impl Iterator for TankIterator<'_> {
    type Item = Pixel<Rgb565>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.position.y >= cvt(self.tank.size.height) {
                return None;
            } else {
                let pv = self.tank.get_point(&self.position);
                let ret = match pv {
                    PointValue::OutOfRange    => None,
                    PointValue::Transparent   => self.some_color(BACKGROUND),
                    PointValue::Opaque(color) => self.some_color(color),
                };

                self.position.x += 1;
                if self.position.x >= cvt(self.tank.size.width) {
                    self.position.x = 0;
                    self.position.y += 1;
                }

                if let Some(_) = ret {
                    return ret;
                }
            }
        }
    }
}

// adapted from
// https://github.com/riscv-rust/longan-nano/blob/master/examples/ferris.rs

#[entry]
fn main() -> ! {
    let dp = pac::Peripherals::take().unwrap();

    // Configure clocks
    let mut rcu = dp
        .RCU
        .configure()
        .ext_hf_clock(8.mhz())
        .sysclk(108.mhz())
        .freeze();
    let mut afio = dp.AFIO.constrain(&mut rcu);

    let gpioa = dp.GPIOA.split(&mut rcu);
    let gpiob = dp.GPIOB.split(&mut rcu);

    let lcd_pins = lcd_pins!(gpioa, gpiob);
    let mut lcd = lcd::configure(dp.SPI0, lcd_pins, &mut afio, &mut rcu);

    // Clear screen
    lcd.clear(rgb565(BACKGROUND)).unwrap();

    let mut fish_tank = FishTank::new(lcd.size(), 0x1badd00d8badf00d);

    loop {
        lcd.draw_iter(TankIterator::new(&fish_tank)).unwrap();
        fish_tank.swim();
    }
}
