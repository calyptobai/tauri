// Copyright 2016-2019 Cargo-Bundle developers <https://github.com/burtonageo/cargo-bundle>
// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::{
  env::{var, var_os},
  ffi::OsString,
  fs::File,
  io::prelude::*,
  path::PathBuf,
  process::Command,
};

use crate::{bundle::common::CommandExt, Settings};
use anyhow::Context;
use log::info;
use serde::Deserialize;

const KEYCHAIN_ID: &str = "tauri-build.keychain";
const KEYCHAIN_PWD: &str = "tauri-build";

// Import certificate from ENV variables.
// APPLE_CERTIFICATE is the p12 certificate base64 encoded.
// By example you can use; openssl base64 -in MyCertificate.p12 -out MyCertificate-base64.txt
// Then use the value of the base64 in APPLE_CERTIFICATE env variable.
// You need to set APPLE_CERTIFICATE_PASSWORD to the password you set when you exported your certificate.
// https://help.apple.com/xcode/mac/current/#/dev154b28f09 see: `Export a signing certificate`
pub fn setup_keychain(
  certificate_encoded: OsString,
  certificate_password: OsString,
) -> crate::Result<()> {
  // we delete any previous version of our keychain if present
  delete_keychain();
  info!("setup keychain from environment variables...");

  let keychain_list_output = Command::new("security")
    .args(["list-keychain", "-d", "user"])
    .output()?;

  let tmp_dir = tempfile::tempdir()?;
  let cert_path = tmp_dir
    .path()
    .join("cert.p12")
    .to_string_lossy()
    .to_string();
  let cert_path_tmp = tmp_dir
    .path()
    .join("cert.p12.tmp")
    .to_string_lossy()
    .to_string();
  let certificate_encoded = certificate_encoded
    .to_str()
    .expect("failed to convert APPLE_CERTIFICATE to string")
    .as_bytes();

  let certificate_password = certificate_password
    .to_str()
    .expect("failed to convert APPLE_CERTIFICATE_PASSWORD to string")
    .to_string();

  // as certificate contain whitespace decoding may be broken
  // https://github.com/marshallpierce/rust-base64/issues/105
  // we'll use builtin base64 command from the OS
  let mut tmp_cert = File::create(cert_path_tmp.clone())?;
  tmp_cert.write_all(certificate_encoded)?;

  Command::new("base64")
    .args(["--decode", "-i", &cert_path_tmp, "-o", &cert_path])
    .output_ok()
    .context("failed to decode certificate")?;

  Command::new("security")
    .args(["create-keychain", "-p", KEYCHAIN_PWD, KEYCHAIN_ID])
    .output_ok()
    .context("failed to create keychain")?;

  Command::new("security")
    .args(["unlock-keychain", "-p", KEYCHAIN_PWD, KEYCHAIN_ID])
    .output_ok()
    .context("failed to set unlock keychain")?;

  Command::new("security")
    .args([
      "import",
      &cert_path,
      "-k",
      KEYCHAIN_ID,
      "-P",
      &certificate_password,
      "-T",
      "/usr/bin/codesign",
      "-T",
      "/usr/bin/pkgbuild",
      "-T",
      "/usr/bin/productbuild",
    ])
    .output_ok()
    .context("failed to import keychain certificate")?;

  Command::new("security")
    .args(["set-keychain-settings", "-t", "3600", "-u", KEYCHAIN_ID])
    .output_ok()
    .context("failed to set keychain settings")?;

  Command::new("security")
    .args([
      "set-key-partition-list",
      "-S",
      "apple-tool:,apple:,codesign:",
      "-s",
      "-k",
      KEYCHAIN_PWD,
      KEYCHAIN_ID,
    ])
    .output_ok()
    .context("failed to set keychain settings")?;

  let current_keychains = String::from_utf8_lossy(&keychain_list_output.stdout)
    .split('\n')
    .map(|line| {
      line
        .trim_matches(|c: char| c.is_whitespace() || c == '"')
        .to_string()
    })
    .filter(|l| !l.is_empty())
    .collect::<Vec<String>>();

  Command::new("security")
    .args(["list-keychain", "-d", "user", "-s"])
    .args(current_keychains)
    .arg(KEYCHAIN_ID)
    .output_ok()
    .context("failed to list keychain")?;

  Ok(())
}

pub fn delete_keychain() {
  // delete keychain if needed and skip any error
  let _ = Command::new("security")
    .arg("delete-keychain")
    .arg(KEYCHAIN_ID)
    .output_ok();
}

pub fn sign(
  path_to_sign: PathBuf,
  identity: &str,
  settings: &Settings,
  is_an_executable: bool,
) -> crate::Result<()> {
  info!(action = "Signing"; "{} with identity \"{}\"", path_to_sign.display(), identity);

  let setup_keychain = if let (Some(certificate_encoded), Some(certificate_password)) = (
    var_os("APPLE_CERTIFICATE"),
    var_os("APPLE_CERTIFICATE_PASSWORD"),
  ) {
    // setup keychain allow you to import your certificate
    // for CI build
    setup_keychain(certificate_encoded, certificate_password)?;
    true
  } else {
    false
  };

  let res = try_sign(
    path_to_sign,
    identity,
    settings,
    is_an_executable,
    setup_keychain,
  );

  if setup_keychain {
    // delete the keychain again after signing
    delete_keychain();
  }

  res
}

fn try_sign(
  path_to_sign: PathBuf,
  identity: &str,
  settings: &Settings,
  is_an_executable: bool,
  tauri_keychain: bool,
) -> crate::Result<()> {
  let mut args = vec!["--force", "-s", identity];

  if tauri_keychain {
    args.push("--keychain");
    args.push(KEYCHAIN_ID);
  }

  if let Some(entitlements_path) = &settings.macos().entitlements {
    info!("using entitlements file at {}", entitlements_path);
    args.push("--entitlements");
    args.push(entitlements_path);
  }

  if is_an_executable {
    args.push("--options");
    args.push("runtime");
  }

  if path_to_sign.is_dir() {
    args.push("--deep");
  }

  Command::new("codesign")
    .args(args)
    .arg(path_to_sign.to_string_lossy().to_string())
    .output_ok()
    .context("failed to sign app")?;

  Ok(())
}

#[derive(Deserialize)]
struct NotarytoolSubmitOutput {
  id: String,
  status: String,
  message: String,
}

pub fn notarize(
  app_bundle_path: PathBuf,
  auth: NotarizeAuth,
  settings: &Settings,
) -> crate::Result<()> {
  let bundle_stem = app_bundle_path
    .file_stem()
    .expect("failed to get bundle filename");

  let tmp_dir = tempfile::tempdir()?;
  let zip_path = tmp_dir
    .path()
    .join(format!("{}.zip", bundle_stem.to_string_lossy()));
  let zip_args = vec![
    "-c",
    "-k",
    "--keepParent",
    "--sequesterRsrc",
    app_bundle_path
      .to_str()
      .expect("failed to convert bundle_path to string"),
    zip_path
      .to_str()
      .expect("failed to convert zip_path to string"),
  ];

  // use ditto to create a PKZip almost identical to Finder
  // this remove almost 99% of false alarm in notarization
  Command::new("ditto")
    .args(zip_args)
    .output_ok()
    .context("failed to zip app with ditto")?;

  // sign the zip file
  if let Some(identity) = &settings.macos().signing_identity {
    sign(zip_path.clone(), identity, settings, false)?;
  };

  let notarize_args = vec![
    "notarytool",
    "submit",
    zip_path
      .to_str()
      .expect("failed to convert zip_path to string"),
    "--wait",
    "--output-format",
    "json",
  ];

  info!(action = "Notarizing"; "{}", app_bundle_path.display());

  let output = Command::new("xcrun")
    .args(notarize_args)
    .notarytool_args(&auth)
    .output_ok()
    .context("failed to upload app to Apple's notarization servers.")?;

  if !output.status.success() {
    return Err(anyhow::anyhow!("failed to notarize app").into());
  }

  let output_str = String::from_utf8_lossy(&output.stdout);
  if let Ok(submit_output) = serde_json::from_str::<NotarytoolSubmitOutput>(&output_str) {
    let log_message = format!(
      "Finished with status {} for id {} ({})",
      submit_output.status, submit_output.id, submit_output.message
    );
    if submit_output.status == "Accepted" {
      log::info!(action = "Notarizing"; "{}", log_message);
      staple_app(app_bundle_path)?;
      Ok(())
    } else {
      Err(anyhow::anyhow!("{log_message}").into())
    }
  } else {
    return Err(
      anyhow::anyhow!("failed to parse notarytool output as JSON: `{output_str}`").into(),
    );
  }
}

fn staple_app(mut app_bundle_path: PathBuf) -> crate::Result<()> {
  let app_bundle_path_clone = app_bundle_path.clone();
  let filename = app_bundle_path_clone
    .file_name()
    .expect("failed to get bundle filename")
    .to_str()
    .expect("failed to convert bundle filename to string");

  app_bundle_path.pop();

  Command::new("xcrun")
    .args(vec!["stapler", "staple", "-v", filename])
    .current_dir(app_bundle_path)
    .output_ok()
    .context("failed to staple app.")?;

  Ok(())
}

pub enum NotarizeAuth {
  AppleId {
    apple_id: String,
    password: String,
  },
  ApiKey {
    key: String,
    key_path: PathBuf,
    issuer: String,
  },
}

pub trait NotarytoolCmdExt {
  fn notarytool_args(&mut self, auth: &NotarizeAuth) -> &mut Self;
}

impl NotarytoolCmdExt for Command {
  fn notarytool_args(&mut self, auth: &NotarizeAuth) -> &mut Self {
    match auth {
      NotarizeAuth::AppleId { apple_id, password } => self
        .arg("--apple-id")
        .arg(apple_id)
        .arg("--password")
        .arg(password),
      NotarizeAuth::ApiKey {
        key,
        key_path,
        issuer,
      } => self
        .arg("--key-id")
        .arg(key)
        .arg("--key")
        .arg(key_path)
        .arg("--issuer")
        .arg(issuer),
    }
  }
}

pub fn notarize_auth() -> crate::Result<NotarizeAuth> {
  match (var_os("APPLE_ID"), var_os("APPLE_PASSWORD")) {
    (Some(apple_id), Some(apple_password)) => {
      let apple_id = apple_id
        .to_str()
        .expect("failed to convert APPLE_ID to string")
        .to_string();
      let password = apple_password
        .to_str()
        .expect("failed to convert APPLE_PASSWORD to string")
        .to_string();
      Ok(NotarizeAuth::AppleId { apple_id, password })
    }
    _ => {
      match (var_os("APPLE_API_KEY"), var_os("APPLE_API_ISSUER"), var("APPLE_API_KEY_PATH")) {
        (Some(api_key), Some(api_issuer), Ok(key_path)) => {
          let key = api_key.to_str().expect("failed to convert APPLE_API_KEY to string").to_string();
          let issuer = api_issuer.to_str().expect("failed to convert APPLE_API_ISSUER to string").to_string();
          Ok(NotarizeAuth::ApiKey { key, key_path: key_path.into(), issuer })
        },
        _ => Err(anyhow::anyhow!("no APPLE_ID & APPLE_PASSWORD or APPLE_API_KEY & APPLE_API_ISSUER & APPLE_API_KEY_PATH environment variables found").into())
      }
    }
  }
}
