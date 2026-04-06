use std::path::PathBuf;
use std::sync::Arc;

use cdk::nuts::CurrencyUnit;
use cdk::nuts::nut00::KnownMethod;
use cdk::wallet::{SendOptions, Wallet};
use cdk::Amount;
use slint::SharedString;
use tokio::sync::Mutex;

slint::include_modules!();

const MINT_URL: &str = "https://mint.minibits.cash/Bitcoin";

struct AppWallet {
    wallet: Wallet,
    current_quote: Option<cdk::wallet::MintQuote>,
}

fn generate_qr_png(data: &str) -> Result<Vec<u8>, String> {
    use image::Luma;
    use qrcode::QrCode;

    // Try normal error correction first, fall back to low
    let code = QrCode::with_error_correction_level(data, qrcode::EcLevel::L)
        .map_err(|e| format!("QR error: {}", e))?;

    let img = code
        .render::<Luma<u8>>()
        .quiet_zone(true)
        .min_dimensions(400, 400)
        .build();

    let mut buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut buf);
    image::ImageEncoder::write_image(
        encoder,
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::L8,
    )
    .map_err(|e| format!("PNG error: {}", e))?;
    Ok(buf)
}

fn png_to_slint_image(png_data: &[u8]) -> slint::Image {
    let img = image::load_from_memory_with_format(png_data, image::ImageFormat::Png).unwrap();
    let rgba = img.to_rgba8();
    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
    );
    slint::Image::from_rgba8(buf)
}

async fn create_wallet() -> Result<Wallet, Box<dyn std::error::Error + Send + Sync>> {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cashu-pixel");
    std::fs::create_dir_all(&data_dir)?;

    let db_path = data_dir.join("wallet.db");
    let localstore = Arc::new(
        cdk_sqlite::wallet::WalletSqliteDatabase::new(&db_path).await?,
    );

    let seed_path = data_dir.join("seed");
    let seed: [u8; 64] = if seed_path.exists() {
        let bytes = std::fs::read(&seed_path)?;
        let mut seed = [0u8; 64];
        seed.copy_from_slice(&bytes[..64]);
        seed
    } else {
        let mut seed = [0u8; 64];
        rand::Rng::fill(&mut rand::thread_rng(), &mut seed);
        std::fs::write(&seed_path, &seed)?;
        seed
    };

    let wallet = Wallet::new(MINT_URL, CurrencyUnit::Sat, localstore, seed, None)?;
    wallet.recover_incomplete_sagas().await?;

    Ok(wallet)
}

fn update_ui(
    ui_weak: &slint::Weak<MainApp>,
    f: impl FnOnce(&MainApp) + Send + 'static,
) {
    let ui_w = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_w.upgrade() {
            f(&ui);
        }
    });
}

fn spawn_wallet_task(
    rt: &tokio::runtime::Handle,
    wallet_ref: Arc<Mutex<Option<AppWallet>>>,
    ui_weak: slint::Weak<MainApp>,
    task: impl std::future::Future<Output = ()> + Send + 'static,
) {
    rt.spawn(task);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = MainApp::new()?;
    let ui_weak = ui.as_weak();

    let app_wallet: Arc<Mutex<Option<AppWallet>>> = Arc::new(Mutex::new(None));

    // Shared tokio runtime on a background thread
    let rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .unwrap(),
    );

    // Init wallet
    let wallet_ref = app_wallet.clone();
    let ui_w = ui_weak.clone();
    rt.spawn(async move {
        match create_wallet().await {
            Ok(wallet) => {
                let balance: u64 = wallet.total_balance().await.unwrap_or(Amount::from(0)).into();
                let mut w = wallet_ref.lock().await;
                *w = Some(AppWallet {
                    wallet,
                    current_quote: None,
                });
                drop(w);
                update_ui(&ui_w, move |ui| {
                    let state = ui.global::<WalletState>();
                    state.set_balance(SharedString::from(format!("{}", balance)));
                    state.set_mint_url(SharedString::from(
                        MINT_URL.replace("https://", "").split('/').next().unwrap_or(MINT_URL),
                    ));
                });
            }
            Err(e) => {
                let msg = format!("Init error: {}", e);
                update_ui(&ui_w, move |ui| {
                    ui.global::<WalletState>().set_status(SharedString::from(msg));
                });
            }
        }
    });

    // Mint quote
    let wallet_ref = app_wallet.clone();
    let ui_w = ui_weak.clone();
    let rt_handle = rt.handle().clone();
    ui.global::<WalletState>().on_request_mint_quote(move |amount| {
        let wallet_ref = wallet_ref.clone();
        let ui_w = ui_w.clone();
        rt_handle.spawn(async move {
            let mut w = wallet_ref.lock().await;
            if let Some(ref mut app) = *w {
                match app
                    .wallet
                    .mint_quote(KnownMethod::Bolt11, Some(Amount::from(amount as u64)), None, None)
                    .await
                {
                    Ok(quote) => {
                        let invoice = quote.request.clone();
                        app.current_quote = Some(quote);
                        drop(w);
                        match generate_qr_png(&invoice) {
                            Ok(png) => {
                                let status = format!("Pay {} sat", amount);
                                update_ui(&ui_w, move |ui| {
                                    let state = ui.global::<WalletState>();
                                    state.set_qr_image(png_to_slint_image(&png));
                                    state.set_qr_ready(true);
                                    state.set_status(SharedString::from(status));
                                    state.set_show_check_payment(true);
                                    state.set_current_screen(4);
                                });
                            }
                            Err(msg) => {
                                update_ui(&ui_w, move |ui| {
                                    ui.global::<WalletState>().set_status(SharedString::from(msg));
                                });
                            }
                        }
                    }
                    Err(e) => {
                        let msg = format!("Error: {}", e);
                        update_ui(&ui_w, move |ui| {
                            ui.global::<WalletState>().set_status(SharedString::from(msg));
                        });
                    }
                }
            }
        });
    });

    // Check mint payment
    let wallet_ref = app_wallet.clone();
    let ui_w = ui_weak.clone();
    let rt_handle = rt.handle().clone();
    ui.global::<WalletState>().on_check_mint_payment(move || {
        let wallet_ref = wallet_ref.clone();
        let ui_w = ui_w.clone();
        rt_handle.spawn(async move {
            let mut w = wallet_ref.lock().await;
            if let Some(ref mut app) = *w {
                if let Some(quote) = app.current_quote.take() {
                    match app.wallet.mint(&quote.id, Default::default(), None).await {
                        Ok(_proofs) => {
                            let bal: u64 = app.wallet.total_balance().await.unwrap_or(Amount::from(0)).into();
                            update_ui(&ui_w, move |ui| {
                                let state = ui.global::<WalletState>();
                                state.set_balance(SharedString::from(format!("{}", bal)));
                                state.set_status(SharedString::from(format!("Minted! Balance: {} sat", bal)));
                                state.set_show_check_payment(false);
                            });
                        }
                        Err(e) => {
                            let msg = format!("{}", e);
                            let is_pending = msg.to_lowercase().contains("not paid")
                                || msg.to_lowercase().contains("pending")
                                || msg.to_lowercase().contains("unpaid");
                            if is_pending {
                                app.current_quote = Some(quote);
                            }
                            let display_msg = if is_pending {
                                "Not paid yet - tap again".to_string()
                            } else {
                                format!("Error: {}", msg)
                            };
                            update_ui(&ui_w, move |ui| {
                                ui.global::<WalletState>()
                                    .set_status(SharedString::from(display_msg));
                            });
                        }
                    }
                }
            }
        });
    });

    // Send token
    let wallet_ref = app_wallet.clone();
    let ui_w = ui_weak.clone();
    let rt_handle = rt.handle().clone();
    ui.global::<WalletState>().on_request_send(move |amount| {
        let wallet_ref = wallet_ref.clone();
        let ui_w = ui_w.clone();
        rt_handle.spawn(async move {
            let mut w = wallet_ref.lock().await;
            if let Some(ref mut app) = *w {
                match app
                    .wallet
                    .prepare_send(Amount::from(amount as u64), SendOptions::default())
                    .await
                {
                    Ok(prepared) => match prepared.confirm(None).await {
                        Ok(token) => {
                            let token_str = token.to_string();
                            let bal: u64 = app.wallet.total_balance().await.unwrap_or(Amount::from(0)).into();
                            drop(w);
                            match generate_qr_png(&token_str) {
                                Ok(png) => {
                                    let status = format!("Scan to receive {} sat", amount);
                                    update_ui(&ui_w, move |ui| {
                                        let state = ui.global::<WalletState>();
                                        state.set_balance(SharedString::from(format!("{}", bal)));
                                        state.set_qr_image(png_to_slint_image(&png));
                                        state.set_qr_ready(true);
                                        state.set_status(SharedString::from(status));
                                        state.set_show_check_payment(false);
                                        state.set_current_screen(4);
                                    });
                                }
                                Err(_) => {
                                    // Token too large for QR — show the token as text instead
                                    let short = format!("{}...", &token_str[..60]);
                                    update_ui(&ui_w, move |ui| {
                                        let state = ui.global::<WalletState>();
                                        state.set_balance(SharedString::from(format!("{}", bal)));
                                        state.set_status(SharedString::from(
                                            format!("Token too large for QR. Token: {}", short),
                                        ));
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            let msg = format!("Error: {}", e);
                            update_ui(&ui_w, move |ui| {
                                ui.global::<WalletState>().set_status(SharedString::from(msg));
                            });
                        }
                    },
                    Err(e) => {
                        let msg = format!("Error: {}", e);
                        update_ui(&ui_w, move |ui| {
                            ui.global::<WalletState>().set_status(SharedString::from(msg));
                        });
                    }
                }
            }
        });
    });

    // BLE receive (placeholder)
    let ui_w = ui_weak.clone();
    ui.global::<WalletState>().on_receive_ble(move || {
        let ui_w = ui_w.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_w.upgrade() {
                ui.global::<WalletState>()
                    .set_status(SharedString::from("BLE not yet available"));
            }
        });
    });

    ui.run()?;
    Ok(())
}
