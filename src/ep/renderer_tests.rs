use super::*;
use std::sync::mpsc;

#[test]
fn test_engine_command_definition() {
    let (tx, _rx) = mpsc::channel::<EngineCommand>();
    let cmd = EngineCommand::ViewportUpdate {
        width: 800.0,
        height: 600.0,
        mx: 0.0,
        my: 0.0,
    };
    tx.send(cmd).unwrap();
}
