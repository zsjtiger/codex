/** Minimal WebSocket wrapper. Reconnection and JSON-RPC handling come later. */
export function openSocket(
  onMessage: (data: string) => void,
  onClose: () => void,
): WebSocket {
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  const socket = new WebSocket(`${scheme}://${location.host}/ws`);
  socket.onmessage = (event) => onMessage(String(event.data));
  socket.onclose = onClose;
  return socket;
}
