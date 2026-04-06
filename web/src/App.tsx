import { useState, useEffect, useCallback } from "react";
import {
  Button,
  Card,
  Badge,
  NervToastProvider,
  MonitorOverlay,
  Divider,
} from "@mdrbx/nerv-ui";

type Screen = "home" | "mint" | "send" | "receive" | "qr" | "settings";

const AMOUNTS_MINT = [21, 100, 500, 1000, 5000, 10000];
const AMOUNTS_SEND = [10, 21, 50, 100, 500, 1000];

function App() {
  const [screen, setScreen] = useState<Screen>("home");
  const [balance, setBalance] = useState(0);
  const [mintUrl, setMintUrl] = useState("");
  const [selectedAmount, setSelectedAmount] = useState(0);
  const [status, setStatus] = useState("");
  const [loading, setLoading] = useState(false);
  const [qrImage, setQrImage] = useState("");
  const [showCheckPayment, setShowCheckPayment] = useState(false);
  const [_tokenText, setTokenText] = useState("");

  const fetchBalance = useCallback(async () => {
    try {
      const res = await fetch("/api/balance");
      const data = await res.json();
      setBalance(data.balance);
      setMintUrl(data.mint_url.replace("https://", "").split("/")[0]);
    } catch {
      setStatus("OFFLINE");
    }
  }, []);

  useEffect(() => {
    fetchBalance();
  }, [fetchBalance]);

  const handleMintQuote = async () => {
    if (selectedAmount <= 0) return;
    setLoading(true);
    setStatus("GENERATING INVOICE...");
    try {
      const res = await fetch("/api/mint/quote", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ amount: selectedAmount }),
      });
      const data = await res.json();
      if (data.error) {
        setStatus(`ERROR: ${data.error}`);
      } else {
        setQrImage(data.qr_png_base64);
        setShowCheckPayment(true);
        setStatus(`PAY ${selectedAmount} SAT`);
        setScreen("qr");
      }
    } catch (e) {
      setStatus("NETWORK ERROR");
    }
    setLoading(false);
  };

  const handleCheckPayment = async () => {
    setLoading(true);
    setStatus("CHECKING PAYMENT...");
    try {
      const res = await fetch("/api/mint/check", { method: "POST" });
      const data = await res.json();
      if (data.error) {
        setStatus(data.error === "NOT_PAID" ? "NOT PAID YET — TAP AGAIN" : `ERROR: ${data.error}`);
      } else {
        setBalance(data.balance);
        setStatus(`MINTED! BALANCE: ${data.balance} SAT`);
        setShowCheckPayment(false);
      }
    } catch {
      setStatus("NETWORK ERROR");
    }
    setLoading(false);
  };

  const handleSend = async () => {
    if (selectedAmount <= 0) return;
    setLoading(true);
    setStatus("GENERATING TOKEN...");
    try {
      const res = await fetch("/api/send", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ amount: selectedAmount }),
      });
      const data = await res.json();
      if (data.error) {
        setStatus(`ERROR: ${data.error}`);
      } else {
        setBalance(data.balance);
        if (data.qr_png_base64) {
          setQrImage(data.qr_png_base64);
        }
        setTokenText(data.token);
        setShowCheckPayment(false);
        setStatus(`SCAN TO RECEIVE ${selectedAmount} SAT`);
        setScreen("qr");
      }
    } catch {
      setStatus("NETWORK ERROR");
    }
    setLoading(false);
  };

  const goHome = () => {
    setScreen("home");
    setStatus("");
    setSelectedAmount(0);
    fetchBalance();
  };

  return (
    <NervToastProvider>
      <div className="relative w-[720px] h-[720px] bg-black overflow-hidden">
        <MonitorOverlay color="orange" density="sparse" />

        {/* ═══ HOME ═══ */}
        {screen === "home" && (
          <div className="flex flex-col h-full p-4 gap-3">
            {/* Header */}
            <div className="flex items-center gap-2 border-b border-nerv-orange/30 pb-2">
              <div className="w-1 h-6 bg-nerv-orange" />
              <span className="text-nerv-orange text-sm font-bold tracking-[0.2em]">
                NERV // CASHU TERMINAL
              </span>
              <span className="ml-auto text-nerv-mid-gray text-xs">V0.1.0</span>
            </div>

            {/* Balance */}
            <Card variant="default" className="flex-none">
              <div className="text-center py-4">
                <div className="text-nerv-orange text-xs font-bold tracking-[0.3em] mb-2">
                  B A L A N C E
                </div>
                <div className="text-nerv-green text-5xl font-bold tracking-wider">
                  {balance.toLocaleString()}
                </div>
                <div className="text-nerv-green text-lg font-bold tracking-[0.2em] mt-1">
                  SAT
                </div>
                <div className="text-nerv-mid-gray text-xs mt-2 tracking-wider">
                  MINT: {mintUrl}
                </div>
              </div>
            </Card>

            <Divider color="orange" />

            {/* Action Buttons */}
            <div className="flex gap-3">
              <Button
                variant="danger"
                size="lg"
                className="flex-1"
                onClick={() => { setScreen("send"); setSelectedAmount(0); setStatus(""); }}
              >
                S E N D
              </Button>
              <Button
                variant="primary"
                size="lg"
                className="flex-1"
                onClick={() => { setScreen("receive"); setStatus(""); }}
              >
                R E C E I V E
              </Button>
            </div>

            <Button
              variant="primary"
              size="lg"
              onClick={() => { setScreen("mint"); setSelectedAmount(0); setStatus(""); }}
            >
              M I N T
            </Button>

            <Button
              variant="ghost"
              size="md"
              onClick={() => setScreen("settings")}
            >
              SETTINGS
            </Button>

            <div className="mt-auto" />

            {/* Status Bar */}
            <div className="flex items-center gap-2 border-t border-nerv-orange/20 pt-2">
              <div className="w-1 h-4 bg-nerv-orange/30" />
              <Badge variant={status ? "warning" : "success"} label={status || "SYSTEM READY"} />
            </div>
          </div>
        )}

        {/* ═══ MINT ═══ */}
        {screen === "mint" && (
          <div className="flex flex-col h-full p-4 gap-3">
            <div className="flex items-center gap-2 border-b border-nerv-orange/30 pb-2">
              <button onClick={goHome} className="text-nerv-orange/60 text-lg">◄</button>
              <div className="w-1 h-6 bg-nerv-amber" />
              <span className="text-nerv-amber text-sm font-bold tracking-[0.2em]">
                MINT // DEPOSIT
              </span>
              <Badge variant="info" className="ml-auto" label="BOLT11" />
            </div>

            {/* Amount display */}
            <Card variant="hud" className="flex-none">
              <div className="text-center py-3">
                <div className="text-nerv-green text-4xl font-bold">
                  {selectedAmount > 0 ? `${selectedAmount.toLocaleString()} SAT` : "SELECT AMOUNT"}
                </div>
              </div>
            </Card>

            {/* Amount grid */}
            <div className="grid grid-cols-3 gap-2">
              {AMOUNTS_MINT.map((amt) => (
                <Button
                  key={amt}
                  variant={selectedAmount === amt ? "primary" : "ghost"}
                  size="lg"
                  onClick={() => setSelectedAmount(amt)}
                >
                  {amt >= 1000 ? `${amt / 1000}K` : amt}
                </Button>
              ))}
            </div>

            <Button
              variant="primary"
              size="lg"
              onClick={handleMintQuote}
              disabled={selectedAmount <= 0 || loading}
            >
              {loading ? "GENERATING..." : "CREATE LIGHTNING INVOICE"}
            </Button>

            <div className="mt-auto" />
            <div className="flex items-center gap-2 border-t border-nerv-orange/20 pt-2">
              <div className="w-1 h-4 bg-nerv-amber/30" />
              <Badge variant={status ? "warning" : "info"} label={status || "SELECT AMOUNT TO MINT"} />
            </div>
          </div>
        )}

        {/* ═══ SEND ═══ */}
        {screen === "send" && (
          <div className="flex flex-col h-full p-4 gap-3">
            <div className="flex items-center gap-2 border-b border-nerv-red/30 pb-2">
              <button onClick={goHome} className="text-nerv-orange/60 text-lg">◄</button>
              <div className="w-1 h-6 bg-nerv-red" />
              <span className="text-nerv-red text-sm font-bold tracking-[0.2em]">
                SEND // TOKEN
              </span>
              <Badge variant="danger" className="ml-auto" label="OFFLINE" />
            </div>

            <Card variant="alert" className="flex-none">
              <div className="text-center py-3">
                <div className="text-nerv-green text-4xl font-bold">
                  {selectedAmount > 0 ? `${selectedAmount.toLocaleString()} SAT` : "SELECT AMOUNT"}
                </div>
              </div>
            </Card>

            <div className="grid grid-cols-3 gap-2">
              {AMOUNTS_SEND.map((amt) => (
                <Button
                  key={amt}
                  variant={selectedAmount === amt ? "danger" : "ghost"}
                  size="lg"
                  onClick={() => setSelectedAmount(amt)}
                >
                  {amt >= 1000 ? `${amt / 1000}K` : amt}
                </Button>
              ))}
            </div>

            <Button
              variant="danger"
              size="lg"
              onClick={handleSend}
              disabled={selectedAmount <= 0 || loading}
            >
              {loading ? "GENERATING..." : "SHOW QR CODE"}
            </Button>

            <div className="mt-auto" />
            <div className="flex items-center gap-2 border-t border-nerv-red/20 pt-2">
              <div className="w-1 h-4 bg-nerv-red/30" />
              <Badge variant={status ? "danger" : "info"} label={status || "SELECT AMOUNT TO SEND"} />
            </div>
          </div>
        )}

        {/* ═══ RECEIVE ═══ */}
        {screen === "receive" && (
          <div className="flex flex-col h-full p-4 gap-3">
            <div className="flex items-center gap-2 border-b border-nerv-cyan/30 pb-2">
              <button onClick={goHome} className="text-nerv-orange/60 text-lg">◄</button>
              <div className="w-1 h-6 bg-nerv-cyan" />
              <span className="text-nerv-cyan text-sm font-bold tracking-[0.2em]">
                RECEIVE // INCOMING
              </span>
              <Badge variant="info" className="ml-auto" label="BLE" />
            </div>

            <div className="flex-1 flex flex-col items-center justify-center gap-6">
              <Button
                variant="primary"
                size="lg"
                className="w-full py-8"
                onClick={() => setStatus("BLE NOT YET AVAILABLE")}
              >
                RECEIVE VIA BLUETOOTH
              </Button>

              <div className="text-nerv-green/70 text-sm font-bold tracking-[0.15em]">
                {status || "TAP TO START LISTENING"}
              </div>
            </div>

            <div className="flex items-center gap-2 border-t border-nerv-cyan/20 pt-2">
              <div className="w-1 h-4 bg-nerv-cyan/30" />
              <Badge variant="info" label={status || "STANDBY"} />
            </div>
          </div>
        )}

        {/* ═══ QR VIEW ═══ */}
        {screen === "qr" && (
          <div className="flex flex-col h-full p-4 gap-2">
            <div className="flex items-center gap-2 border-b border-nerv-orange/30 pb-2">
              <div className="w-1 h-6 bg-nerv-orange" />
              <span className="text-nerv-orange text-sm font-bold tracking-[0.2em]">
                QR // DATA OUTPUT
              </span>
            </div>

            {/* QR Display */}
            <div className="flex-1 flex items-center justify-center border border-nerv-orange/30 bg-black">
              {qrImage ? (
                <img
                  src={`data:image/png;base64,${qrImage}`}
                  alt="QR Code"
                  className="max-w-full max-h-full object-contain p-2"
                  style={{ imageRendering: "pixelated" }}
                />
              ) : (
                <div className="text-nerv-orange/40 text-sm">NO QR DATA</div>
              )}
            </div>

            {/* Status */}
            <div className="text-center">
              <Badge variant={showCheckPayment ? "warning" : "success"} label={status} />
            </div>

            {/* Buttons */}
            {showCheckPayment && (
              <Button
                variant="primary"
                size="lg"
                onClick={handleCheckPayment}
                disabled={loading}
              >
                {loading ? "CHECKING..." : "CHECK PAYMENT"}
              </Button>
            )}

            <Button variant="ghost" size="md" onClick={goHome}>
              DONE
            </Button>
          </div>
        )}

        {/* ═══ SETTINGS ═══ */}
        {screen === "settings" && (
          <div className="flex flex-col h-full p-4 gap-3">
            <div className="flex items-center gap-2 border-b border-nerv-orange/30 pb-2">
              <button onClick={goHome} className="text-nerv-orange/60 text-lg">◄</button>
              <div className="w-1 h-6 bg-nerv-orange" />
              <span className="text-nerv-orange text-sm font-bold tracking-[0.2em]">
                SETTINGS // CONFIG
              </span>
            </div>

            <Card variant="default" className="flex-none">
              <div className="p-3">
                <div className="text-nerv-orange text-xs font-bold tracking-[0.2em] mb-1">BALANCE</div>
                <div className="text-nerv-green text-3xl font-bold">{balance.toLocaleString()} SAT</div>
              </div>
            </Card>

            <Divider color="orange" />

            <Card variant="default" className="flex-none">
              <div className="p-3">
                <div className="text-nerv-orange text-xs font-bold tracking-[0.2em] mb-1">MINT URL</div>
                <div className="text-nerv-white text-sm">{mintUrl}</div>
              </div>
            </Card>

            <Divider color="orange" />

            <Card variant="default" className="flex-none">
              <div className="p-3">
                <div className="text-nerv-orange text-xs font-bold tracking-[0.2em] mb-1">VERSION</div>
                <div className="text-nerv-mid-gray text-xs">CASHU PIXEL V0.1.0 // CDK 0.15 // NERV-UI</div>
              </div>
            </Card>

            <div className="mt-auto" />
            <div className="flex items-center gap-2 border-t border-nerv-orange/20 pt-2">
              <div className="w-1 h-4 bg-nerv-orange/30" />
              <Badge variant="success" label="SYSTEM NOMINAL" />
            </div>
          </div>
        )}
      </div>
    </NervToastProvider>
  );
}

export default App;
