use caliber_synthetic_rust::{Command, SAMPLE_SNARE, SyntheticBackend};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut backend = SyntheticBackend::new()?;
    let state = backend.dispatch(Command::SelectSample {
        sample_id: SAMPLE_SNARE,
        based_on_revision: 1,
    })?;
    let waveform = state.waveform.as_ref().expect("selected sample waveform");
    println!(
        "synthetic-rust: revision={} samples={} waveform_bytes={} resources={}",
        state.revision,
        state.visible_samples.len(),
        backend.map_waveform(waveform)?.bytes().len(),
        backend.live_resource_count()
    );
    backend.publish_meter(0.25, 0.5)?;
    println!("meter: {:?}", backend.read_meter()?);
    println!("trace entries: {}", backend.trace().len());
    Ok(())
}
