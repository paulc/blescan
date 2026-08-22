const CH9143_NOTIFY_UUID = "0xfff1::utf8";
const CH9143_WRITE_UUID = "0xfff2::utf8";
const BAUD = 115200;
const CHUNK_SIZE = 63;

const chunk = (s, n) => {
  const chunks = [];
  for (let i = 0; i < s.length; i += n) {
    chunks.push(s.slice(i, i + n));
  }
  return chunks;
};

// Connect to CH9143 BLE/Serial Bridge and connect STDIN/STDOUT
//
// `name`: device name (default CH9143BLE2U)
// `noResp`: use WriteNoResponse (significantly lower latency) with rate limiting
//
const ch9143 = async ({ name = "CH9143BLE2U", noResp = false } = {}) => {
  // Get first matching instance from scanner
  const scanner = scan({ name });
  const dev = (await scanner.next()).value;

  // Close scanner
  scanner.close();

  // Connect to device & enumerate
  await dev.connect();
  await dev.enumerate();
  if (noResp) {
    console.log_err(">> CONNECTED [WriteNoResp]");
  } else {
    console.log_err(">> CONNECTED");
  }

  // Subscribe to notify characteristic
  await dev.subscribe([CH9143_NOTIFY_UUID], true);

  // Install notification handler
  const n = dev.on_notification((n) => {
    __print(n.value[0]);
  });

  while (true) {
    const line = await __readline_async();
    if (line === undefined) {
      break;
    }
    for (const c of chunk(line + "\r\n", CHUNK_SIZE)) {
      const r = await dev.write(CH9143_WRITE_UUID, c, noResp);
      if (r !== undefined) {
        console.log_err("!! BLE Error:", r);
      }
      if (noResp) {
        // Rate Limit
        await __sleep((c.length * 10 / BAUD) * 1.2);
      }
    }
  }

  if (noResp) {
    // Let events drain
    await __sleep(1);
  }
  await n.stop();
  await dev.disconnect();
};
