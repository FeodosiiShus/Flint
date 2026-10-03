use std::io::Cursor;

use anyhow::{Context as _, Result};
use collections::HashMap;
use gpui::{App, BorrowAppContext, Global};
use rodio::{
    ChannelCount, Decoder, DeviceSinkBuilder, MixerDeviceSink, SampleRate, Source, mixer::Mixer,
    nz, source::Buffered,
};
use util::ResultExt;

const SAMPLE_RATE: SampleRate = nz!(48000);
const CHANNEL_COUNT: ChannelCount = nz!(2);

#[derive(Debug, Copy, Clone, Eq, Hash, PartialEq)]
pub enum Sound {
    AgentDone,
}

impl Sound {
    fn file(&self) -> &'static str {
        match self {
            Self::AgentDone => "agent_done",
        }
    }

    fn asset_path(&self) -> String {
        format!("sounds/{}.wav", self.file())
    }
}

#[derive(Default)]
pub struct Audio {
    output: Option<(MixerDeviceSink, Mixer)>,
    source_cache: HashMap<Sound, Buffered<Decoder<Cursor<Vec<u8>>>>>,
}

impl Global for Audio {}

impl Audio {
    pub fn play_sound(sound: Sound, cx: &mut App) {
        cx.update_default_global(|this: &mut Self, cx| {
            let source = this.sound_source(sound, cx).log_err()?;
            let output_mixer = this
                .ensure_output_exists()
                .context("Could not get output mixer")
                .log_err()?;
            output_mixer.add(source);
            Some(())
        });
    }

    fn ensure_output_exists(&mut self) -> Result<&Mixer> {
        let (_, mixer) = match &mut self.output {
            Some(output) => output,
            output @ None => output.insert(open_output_stream()?),
        };
        Ok(mixer)
    }

    fn sound_source(&mut self, sound: Sound, cx: &App) -> Result<impl Source + use<>> {
        if let Some(wav) = self.source_cache.get(&sound) {
            return Ok(wav.clone());
        }

        let path = sound.asset_path();
        let bytes = cx
            .asset_source()
            .load(&path)?
            .with_context(|| format!("No asset available for path {path}"))?
            .into_owned();
        let source = Decoder::new(Cursor::new(bytes))?.buffered();
        self.source_cache.insert(sound, source.clone());
        Ok(source)
    }
}

fn open_output_stream() -> Result<(MixerDeviceSink, Mixer)> {
    let mut output_handle =
        DeviceSinkBuilder::open_default_sink().context("Could not open default output stream")?;
    output_handle.log_on_drop(false);
    log::info!("Output stream: {:?}", output_handle);

    let (output_mixer, source) = rodio::mixer::mixer(CHANNEL_COUNT, SAMPLE_RATE);
    output_mixer.add(rodio::source::Zero::new(CHANNEL_COUNT, SAMPLE_RATE));
    output_handle.mixer().add(source);

    Ok((output_handle, output_mixer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AssetSource as _;

    #[test]
    fn every_sound_is_bundled_as_a_decodable_wav() {
        for sound in [Sound::AgentDone] {
            let path = sound.asset_path();
            let bytes = assets::Assets
                .load(&path)
                .unwrap_or_else(|error| panic!("loading {path} failed: {error:#}"))
                .unwrap_or_else(|| panic!("{path} is not bundled"))
                .into_owned();
            let decoder = Decoder::new(Cursor::new(bytes))
                .unwrap_or_else(|error| panic!("{path} is not a decodable wav: {error}"));
            assert!(decoder.sample_rate().get() > 0, "{path} has no sample rate");
        }
    }
}
