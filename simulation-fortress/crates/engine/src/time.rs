use bevy_ecs::prelude::World;
use bevy_ecs::resource::Resource;

pub type Tick = u64;

/// Simulation tick counter. Scenarios can also configure
/// `ticks_per_minute` so absolute wall-clock can be derived from
/// `tick`. Defaults to 1 tick = 1 in-game minute.
#[derive(Resource, Clone, Copy, Debug)]
pub struct Clock {
    pub tick: Tick,
    pub ticks_per_minute: u32,
    /// Initial in-game time as minutes since midnight (0..1440).
    pub start_minute: u32,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            tick: 0,
            ticks_per_minute: 1,
            // Start at 6:30 AM by default — early morning is a useful
            // baseline for most scenarios.
            start_minute: 6 * 60 + 30,
        }
    }
}

impl Clock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&mut self) -> Tick {
        self.tick += 1;
        self.tick
    }

    /// Total in-game minutes since midnight (wraps at 1440).
    pub fn minute_of_day(&self) -> u32 {
        let elapsed = self.tick / self.ticks_per_minute as u64;
        ((self.start_minute as u64 + elapsed) % (24 * 60)) as u32
    }

    pub fn hour(&self) -> u32 {
        self.minute_of_day() / 60
    }

    pub fn minute(&self) -> u32 {
        self.minute_of_day() % 60
    }

    /// "06:42" style label.
    pub fn time_label(&self) -> String {
        format!("{:02}:{:02}", self.hour(), self.minute())
    }

    pub fn time_of_day(&self) -> TimeOfDay {
        match self.hour() {
            0..=4 => TimeOfDay::Night,
            5 => TimeOfDay::Dawn,
            6..=7 => TimeOfDay::EarlyMorning,
            8..=11 => TimeOfDay::Morning,
            12..=13 => TimeOfDay::Noon,
            14..=16 => TimeOfDay::Afternoon,
            17..=18 => TimeOfDay::Dusk,
            19..=21 => TimeOfDay::Evening,
            _ => TimeOfDay::Night,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TimeOfDay {
    Night,
    Dawn,
    EarlyMorning,
    Morning,
    Noon,
    Afternoon,
    Dusk,
    Evening,
}

impl TimeOfDay {
    pub fn label(self) -> &'static str {
        match self {
            TimeOfDay::Night => "night",
            TimeOfDay::Dawn => "dawn",
            TimeOfDay::EarlyMorning => "early morning",
            TimeOfDay::Morning => "morning",
            TimeOfDay::Noon => "noon",
            TimeOfDay::Afternoon => "afternoon",
            TimeOfDay::Dusk => "dusk",
            TimeOfDay::Evening => "evening",
        }
    }

    /// Sunlight contribution to ambient lux. Indoor lights add on
    /// top via `LightSource` entities.
    pub fn ambient_lux(self) -> f32 {
        match self {
            TimeOfDay::Night => 5.0,
            TimeOfDay::Dawn => 80.0,
            TimeOfDay::EarlyMorning => 400.0,
            TimeOfDay::Morning => 1200.0,
            TimeOfDay::Noon => 2400.0,
            TimeOfDay::Afternoon => 1600.0,
            TimeOfDay::Dusk => 200.0,
            TimeOfDay::Evening => 30.0,
        }
    }

    /// Multiplier on `Sight.range` — daylight is 1.0, night drops
    /// vision to ~25% (you can still see lit rooms / nearby tiles).
    pub fn sight_multiplier(self) -> f32 {
        match self {
            TimeOfDay::Night => 0.25,
            TimeOfDay::Dawn => 0.55,
            TimeOfDay::EarlyMorning => 0.85,
            TimeOfDay::Morning | TimeOfDay::Noon | TimeOfDay::Afternoon => 1.0,
            TimeOfDay::Dusk => 0.65,
            TimeOfDay::Evening => 0.40,
        }
    }
}

/// Weather across the whole world. Scenarios set this, the
/// `weather_system` does periodic side-effects (rain leaves water
/// coatings outdoors, snow leaves snow coatings, storms add wind).
#[derive(Resource, Clone, Debug, Default)]
pub struct Weather {
    pub kind: WeatherKind,
    /// 0..=1, intensity of the current weather (drizzle vs.
    /// downpour, light snow vs. blizzard).
    pub intensity: f32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum WeatherKind {
    #[default]
    Clear,
    Cloudy,
    Rain,
    Snow,
    Storm,
    Fog,
}

impl WeatherKind {
    pub fn label(self) -> &'static str {
        match self {
            WeatherKind::Clear => "clear",
            WeatherKind::Cloudy => "cloudy",
            WeatherKind::Rain => "rain",
            WeatherKind::Snow => "snow",
            WeatherKind::Storm => "storm",
            WeatherKind::Fog => "fog",
        }
    }

    pub fn dims_outdoor_light(self) -> bool {
        matches!(
            self,
            WeatherKind::Cloudy | WeatherKind::Rain | WeatherKind::Snow | WeatherKind::Storm | WeatherKind::Fog
        )
    }

    /// Multiplier on outdoor ambient lux (1.0 = no dimming).
    pub fn light_multiplier(self) -> f32 {
        match self {
            WeatherKind::Clear => 1.0,
            WeatherKind::Cloudy => 0.7,
            WeatherKind::Fog => 0.5,
            WeatherKind::Rain => 0.55,
            WeatherKind::Snow => 0.6,
            WeatherKind::Storm => 0.35,
        }
    }
}

/// Periodic weather side-effects: rain / snow paint outdoor tiles
/// with water / ice coatings; storms also drop the occasional
/// branch (left for scenarios). Currently a no-op stub the schedule
/// can call once a tick.
pub fn weather_system(_world: &mut World) {
    // Placeholder — concrete coating logic lives in scenarios that
    // know which tiles are "outdoors". When we add Outdoor markers
    // engine-wide, this can move here.
}
