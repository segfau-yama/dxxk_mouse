pub type AudioBuffer<const N: usize> = [i16; N];

#[cfg(test)]
mod tests {
    use super::AudioBuffer;

    #[test]
    fn audio_bufferはpcmサンプル列として扱える() {
        let buffer: AudioBuffer<2> = [120, -120];

        assert_eq!(buffer, [120, -120]);
    }
}
