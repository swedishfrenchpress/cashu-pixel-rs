//! The mints both apps offer under Settings, and the check a mint must pass before either
//! app moves to it. Cashu NERV and Pixel Faucet each include this file as a module.

use std::path::Path;
use std::str::FromStr;

use cdk::mint_url::MintUrl;
use cdk::nuts::nut00::PaymentMethod;
use cdk::nuts::CurrencyUnit;
use cdk::wallet::Wallet;

/// Offered on every device
const KNOWN: &str = include_str!("known-mints.txt");
/// More mints for one device, listed in its app's data directory in the same format
const EXTRA_FILE: &str = "mints.txt";

/// The mints to offer: the known ones, then those in `data_dir/mints.txt`, without repeats
pub fn offered(data_dir: &Path) -> Vec<MintUrl> {
    let extra = std::fs::read_to_string(data_dir.join(EXTRA_FILE)).unwrap_or_default();
    let mut mints: Vec<MintUrl> = Vec::new();
    for mint in parse(KNOWN).chain(parse(&extra)) {
        if !mints.contains(&mint) {
            mints.push(mint);
        }
    }
    mints
}

/// The mint URLs in a list: one per line, `#` starts a comment, `https://` may be left off,
/// and lines that aren't URLs are skipped
fn parse(list: &str) -> impl Iterator<Item = MintUrl> + '_ {
    list.lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let url = if line.contains("://") { line.to_string() } else { format!("https://{}", line) };
            // MintUrl only tidies the text; building a real URL from it is the check
            MintUrl::from_str(&url).ok().filter(|mint| mint.join_paths(&["v1", "info"]).is_ok())
        })
}

/// A mint URL as the screen shows it: host only
pub fn host(mint: &MintUrl) -> String {
    let url = mint.to_string();
    url.trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or(&url).to_string()
}

/// Whether `wallet`'s mint can take over: it answers, and it both issues and pays out sat
/// over Lightning (NUT-04 and NUT-05), which receiving and moving sats rely on
pub async fn check(wallet: &Wallet) -> Result<(), String> {
    let info = wallet.fetch_mint_info().await.map_err(|e| e.to_string())?.ok_or("the mint sent no info")?;
    let (unit, lightning) = (CurrencyUnit::Sat, PaymentMethod::BOLT11);
    if info.nuts.nut04.disabled || info.nuts.nut04.get_settings(&unit, &lightning).is_none() {
        return Err("it doesn't issue sat over Lightning".to_string());
    }
    if info.nuts.nut05.disabled || info.nuts.nut05.get_settings(&unit, &lightning).is_none() {
        return Err("it doesn't pay Lightning invoices in sat".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_mints_all_parse() {
        let known: Vec<MintUrl> = parse(KNOWN).collect();
        let lines = KNOWN.lines().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')).count();
        assert_eq!(known.len(), lines);
        assert_eq!(host(&known[0]), "mint.minibits.cash");
    }

    #[test]
    fn extra_mints_follow_the_known_ones_without_repeats() {
        let dir = std::env::temp_dir().join(format!("mints-test-{:08x}", rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(EXTRA_FILE),
            "# mine\nmint.example.com\n\nhttps://antifiat.cash/   # already known\nnot a url at all\n",
        )
        .unwrap();
        let offered = offered(&dir);
        let known = parse(KNOWN).count();
        assert_eq!(offered.len(), known + 1);
        assert_eq!(host(&offered[known]), "mint.example.com");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Asks the real mints, so it's opt-in: `cargo test -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn every_known_mint_passes_the_check() {
        let dir = std::env::temp_dir().join(format!("mints-live-{:08x}", rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = std::sync::Arc::new(cdk_sqlite::wallet::WalletSqliteDatabase::new(&dir.join("wallet.db")).await.unwrap());
        let mut failed = Vec::new();
        for mint in parse(KNOWN) {
            let wallet = Wallet::new(&mint.to_string(), CurrencyUnit::Sat, store.clone(), [7; 64], None).unwrap();
            match check(&wallet).await {
                Ok(()) => println!("{} passes", mint),
                Err(e) => failed.push(format!("{}: {}", mint, e)),
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(failed.is_empty(), "{:#?}", failed);
    }
}
