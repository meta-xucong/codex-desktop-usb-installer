use std::env;

use usb_license_signer::SignerError;

fn main() {
    match usb_license_signer::run_with_args(env::args_os().skip(1)) {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("usb-license-signer output serialization failed: {error}");
                std::process::exit(1);
            }
        },
        Err(SignerError::HelpRequested(text)) => println!("{text}"),
        Err(error) => {
            eprintln!("usb-license-signer: {error}");
            std::process::exit(if matches!(error, SignerError::OutcomeUnknown(_)) {
                2
            } else {
                1
            });
        }
    }
}
