export class BodyLimitError extends Error {}

// Bound allocation while retaining only one growing buffer and one incoming chunk.
export async function readBoundedBytes(stream, maximum) {
  if (!stream) return null;
  const reader = stream.getReader();
  let buffer = new Uint8Array(Math.min(4096, maximum));
  let length = 0;
  let complete = false;
  try {
    for (let part = await reader.read(); !part.done; part = await reader.read()) {
      const required = length + part.value.byteLength;
      if (required > maximum) throw new BodyLimitError('Body exceeds byte limit');
      if (required > buffer.length) {
        const grown = new Uint8Array(Math.min(maximum, Math.max(required, buffer.length * 2)));
        grown.set(buffer.subarray(0, length));
        buffer = grown;
      }
      buffer.set(part.value, length);
      length = required;
    }
    complete = true;
    return buffer.slice(0, length);
  } finally {
    try {
      if (!complete) await reader.cancel();
    } finally {
      reader.releaseLock();
    }
  }
}
