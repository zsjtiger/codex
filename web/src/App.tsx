import { useEffect, useRef, useState } from "react";
import { openSocket } from "./realtime/socket";

/** Skeleton page: connects to the gateway and shows echoed frames. */
export function App() {
  const [log, setLog] = useState<string[]>([]);
  const [connected, setConnected] = useState(false);
  const socketRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    const socket = openSocket(
      (data) => setLog((prev) => [...prev, `← ${data}`]),
      () => setConnected(false),
    );
    socket.onopen = () => setConnected(true);
    socketRef.current = socket;
    return () => socket.close();
  }, []);

  const send = () => {
    const payload = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize" });
    socketRef.current?.send(payload);
    setLog((prev) => [...prev, `→ ${payload}`]);
  };

  return (
    <main style={{ fontFamily: "system-ui", padding: 16 }}>
      <h1>Codex</h1>
      <p>Gateway: {connected ? "connected" : "disconnected"}</p>
      <button onClick={send} disabled={!connected}>
        Send initialize
      </button>
      <pre>{log.join("\n")}</pre>
    </main>
  );
}
