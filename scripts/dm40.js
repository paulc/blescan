
const DM40_WRITE_UUID = "0000fff1-0000-1000-8000-00805f9b34fb"
const DM40_READ_UUID = "0000fff2-0000-1000-8000-00805f9b34fb"

const CMD_ID= "AF0503080041";
const CMD_READ = "AF0503090040";

const dm40c = async ({showServices = false} = {}) => {
  // Get first matching instance from scanner
  const scanner = scan({name:"DM40C"});
  const dev = (await scanner.next()).value;

  // Close scanner
  scanner.close();

  // Connect to device & enumerate
  console.log(">> CONNECT");
  await dev.connect();
  console.log(">> ENUMERATE");
  await dev.enumerate();
  if (showServices) {
    console.log(JSON.stringify(await dev.snapshot(),null,2))
  }

  // Subscribe to notify characteristic
  await dev.subscribe([DM40_READ_UUID], true);

  // Install notification handler
  const n = dev.on_notification((n) => {
    const now = new Date().toISOString();
    const { value, unit, mode } = parseMeasurement(n.value);
    console.log(`>> NOTIFICATION :: Time: ${now} [${mode}] ${value}${unit}`);
  });

  // Send ID command
  console.log(">> CMD_ID");
  await dev.write(DM40_WRITE_UUID, CMD_ID, true);

  setInterval(() => {
    dev.write(DM40_WRITE_UUID, CMD_READ, true)
  }, 100);

  // Return stop function - cancel subscription & disconnect
  return () => { n.stop(); dev.disconnect() };
}

/**
 * DM40C Measurement Frame Parser
 * ================================
 * Parses the 16-byte "DF 05 03 09 ..." measurement notification frame
 * emitted by the ALIENTEK DM40C multimeter over BLE, given as a hex string.
 *
 * The entire file is a single IIFE that evaluates to the parseMeasurement
 * function — just eval/load this file and call parseMeasurement directly:
 *
 *   const result = parseMeasurement("df050309000040000000000000642a00");
 *   // => { value: "12.345", unit: "V", mode: "DC Voltage", ... }
 */

const parseMeasurement = (function () {

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MEASUREMENT_HEADER = [0xdf, 0x05, 0x03, 0x09];
const MEASUREMENT_FRAME_LEN = 16;
const DEVICE_COUNTS = 60000.0;
const CONTINUITY_ALT_AUX_SCALE_FLAGS = new Set([0x84]);
const CONTINUITY_WRAP_OFFSET = 65520;

// Voltage / diode scale map, keyed by scale_flag (frame[-8] & 0xFE)
const ALT_SCALE_MAP = {
  0x02: [6.0, "V", 1.0, 4],
  0x04: [0.6, "mV", 1e3, 2],
  0x06: [0.6, "mV", 1e3, 2],
  0x08: [6.0, "V", 1.0, 4],
  0x0a: [6.0, "V", 1.0, 4],
  0x12: [6000.0, "V", 1.0, 1],
  0x14: [600.0, "V", 1.0, 2],
  0x16: [60.0, "V", 1.0, 3],
  0x18: [6.0, "V", 1.0, 4],
  0x1a: [6.0, "V", 1.0, 4],
  0x26: [60.0, "V", 1.0, 3],
  0x28: [6.0, "V", 1.0, 4],
  0x2a: [60.0, "V", 1.0, 3],
  0x00: [0.6, "mV", 1e3, 2],
  0x10: [60.0, "V", 1.0, 3],
  0x20: [1000.0, "V", 1.0, 1],
  0x30: [6.0, "V", 1.0, 4],
  0x40: [0.6, "mV", 1e3, 2],
  0x48: [6.0, "V", 1.0, 4],
  0x50: [60.0, "V", 1.0, 3],
  0x58: [600.0, "V", 1.0, 2],
  0x60: [1000.0, "V", 1.0, 1],
  0x68: [6.0, "V", 1.0, 4],
  0x70: [6.0, "V", 1.0, 4],
};

// DCV-only override (some firmware reuses scale flags differently between ACV/DCV)
const VDC_SCALE_OVERRIDE_MAP = {
  0x02: [60.0, "V", 1.0, 3],
  0x04: [6.0, "V", 1.0, 4],
  0x06: [6.0, "V", 1.0, 4],
};

// DCV range flag (frame[5] / mode_flag) — more reliable than scale_flag on some variants
const VDC_MODE_SCALE_MAP = {
  0x00: [0.6, "mV", 1e3, 2],
  0x08: [6.0, "V", 1.0, 4],
  0x10: [60.0, "V", 1.0, 3],
  0x18: [600.0, "V", 1.0, 2],
  0x20: [1000.0, "V", 1.0, 1],
  0x28: [6.0, "V", 1.0, 4],
  0x30: [6.0, "V", 1.0, 4],
};

const AMP_SCALE_MAP = {
  0x02: [6000e-6, "uA", 1e6, 1],
  0x04: [600e-6, "uA", 1e6, 2],
  0x06: [600e-6, "uA", 1e6, 2],
  0x14: [600e-3, "mA", 1e3, 2],
  0x16: [60e-3, "mA", 1e3, 3],
  0x18: [6e-3, "mA", 1e3, 4],
  0x1a: [6e-3, "mA", 1e3, 4],
  0x26: [60.0, "A", 1.0, 3],
  0x28: [6.0, "A", 1.0, 4],
  0x2a: [6.0, "A", 1.0, 4],
};

// Current mode flag (frame[5]) -> scale profile
const AMP_MODE_SCALE_MAP = {
  0x01: [600e-6, "uA", 1e6, 2],
  0x09: [6e-3, "uA", 1e6, 1],
  0x11: [60e-3, "mA", 1e3, 3],
  0x19: [600e-3, "mA", 1e3, 2],
  0x21: [6.0, "A", 1.0, 4],
  0x29: [10.0, "A", 1.0, 3],
  0x41: [600e-6, "uA", 1e6, 2],
  0x49: [6e-3, "uA", 1e6, 1],
  0x51: [60e-3, "mA", 1e3, 3],
  0x59: [600e-3, "mA", 1e3, 2],
  0x61: [6.0, "A", 1.0, 4],
  0x69: [10.0, "A", 1.0, 3],
};

const RES_SCALE_MAP = {
  0x00: [600000.0, "kΩ", 0.001, 2],
  0x02: [6000.0, "Ω", 1.0, 1],
  0x04: [600.0, "Ω", 1.0, 2],
  0x06: [600.0, "Ω", 1.0, 2],
  0x14: [600000.0, "kΩ", 0.001, 2],
  0x16: [60000.0, "kΩ", 0.001, 3],
  0x18: [6000.0, "kΩ", 0.001, 4],
  0x1a: [6000.0, "kΩ", 0.001, 4],
  0x26: [6e7, "MΩ", 1e-6, 3],
  0x28: [6e6, "MΩ", 1e-6, 4],
  0x2a: [6e6, "MΩ", 1e-6, 4],
};

// Resistance mode flag (frame[5]) -> scale profile
const RES_MODE_SCALE_MAP = {
  0x02: [600.0, "Ω", 1.0, 2],
  0x0a: [6000.0, "kΩ", 0.001, 4],
  0x12: [60000.0, "kΩ", 0.001, 3],
  0x1a: [600000.0, "kΩ", 0.001, 2],
  0x22: [6e6, "MΩ", 1e-6, 4],
  0x2a: [6e7, "MΩ", 1e-6, 3],
  0x42: [600.0, "Ω", 1.0, 2],
  0x4a: [6000.0, "kΩ", 0.001, 4],
  0x52: [60000.0, "kΩ", 0.001, 3],
  0x5a: [600000.0, "kΩ", 0.001, 2],
  0x62: [6e6, "MΩ", 1e-6, 4],
  0x6a: [6e7, "MΩ", 1e-6, 3],
};

const FREQ_SCALE_MAP = {
  0x02: [6000.0, "Hz", 1.0, 1],
  0x04: [600.0, "Hz", 1.0, 2],
  0x06: [60.0, "Hz", 1.0, 3],
  0x14: [600000.0, "kHz", 1e-3, 2],
  0x16: [60000.0, "kHz", 1e-3, 3],
  0x18: [6000.0, "kHz", 1e-3, 4],
  0x26: [6000000.0, "kHz", 1e-3, 1],
  0x28: [6000000.0, "MHz", 1e-6, 4],
};

const CAP_SCALE_MAP = {
  0x02: [600e-9, "nF", 1e9, 1],
  0x04: [60e-9, "nF", 1e9, 2],
  0x06: [6e-9, "nF", 1e9, 3],
  0x12: [600e-6, "uF", 1e6, 1],
  0x14: [60e-6, "uF", 1e6, 2],
  0x16: [6e-6, "uF", 1e6, 3],
  0x24: [60e-3, "mF", 1e3, 2],
  0x26: [6e-3, "mF", 1e3, 3],
  0x28: [600e-6, "uF", 1e6, 1],
};

// Fallback raw multiplier per scale_flag when no scale map entry matches.
const FALLBACK_MULT = {
  0x00: 0.00001, 0x02: 0.001, 0x04: 0.001, 0x06: 0.001,
  0x08: 0.0001, 0x0a: 0.001, 0x0e: 0.001,
  0x12: 0.1, 0x14: 0.01, 0x16: 0.001, 0x18: 0.0001,
  0x24: 0.01, 0x26: 10.0, 0x28: 0.0001, 0x2a: 0.001,
};
const FALLBACK_UNIT = {
  VDC: "V", VAC: "V", ADC: "A", AAC: "A",
  RES: "Ω", CONT: "Ω", CAP: "F", FREQ: "Hz",
  DIODE: "V", TEMP: "°C",
};

// Units that should be rendered as fixed-decimal strings (matches device display)
const STRING_UNITS = new Set([
  "mV", "V", "uA", "mA", "A", "Ω", "kΩ", "MΩ",
  "Hz", "kHz", "MHz", "nF", "uF", "mF", "F", "°C", "°F",
]);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Convert a hex string (with or without spaces / "0x" prefixes) to a byte array. */
function hexToBytes(hex) {
  const clean = hex.replace(/0x/gi, "").replace(/[^0-9a-fA-F]/g, "");
  if (clean.length % 2 !== 0) {
    throw new Error("Hex string must have an even number of digits");
  }
  const bytes = new Array(clean.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(clean.substr(i * 2, 2), 16);
  }
  return bytes;
}

/** Find the last occurrence of `needle` within `haystack` (byte arrays). Returns index or -1. */
function lastIndexOfSequence(haystack, needle) {
  for (let i = haystack.length - needle.length; i >= 0; i--) {
    let match = true;
    for (let j = 0; j < needle.length; j++) {
      if (haystack[i + j] !== needle[j]) {
        match = false;
        break;
      }
    }
    if (match) return i;
  }
  return -1;
}

/** Extract the most recent full 16-byte measurement frame from a raw payload. */
function extractMeasurementFrame(payload) {
  const idx = lastIndexOfSequence(payload, MEASUREMENT_HEADER);
  if (idx < 0) return null;
  const end = idx + MEASUREMENT_FRAME_LEN;
  if (end > payload.length) return null;
  return payload.slice(idx, end);
}

/** Decode the DM40 mode flag (frame[5]) into [modeKind, modeName]. */
function decodeMode(modeFlag) {
  const vdcFlags = new Set([0x00, 0x08, 0x10, 0x18, 0x20, 0x28, 0x30]);
  const vacFlags = new Set([0x40, 0x48, 0x50, 0x58, 0x60, 0x68, 0x70]);
  const vdcAcFlags = new Set([0x80, 0x88, 0x90, 0x98, 0xa0, 0xa8, 0xb0]);
  const adcFlags = new Set([0x01, 0x09, 0x11, 0x19, 0x21, 0x29, 0x31, 0x39]);
  const aacFlags = new Set([0x41, 0x49, 0x51, 0x59, 0x61, 0x69, 0x71, 0x79]);
  const adcAcFlags = new Set([0x81, 0x89, 0x91, 0x99, 0xa1, 0xa9, 0xb1, 0xb9]);
  const resistanceFlags = new Set([
    0x02, 0x0a, 0x12, 0x1a, 0x22, 0x2a, 0x32, 0x42, 0x4a, 0x52, 0x5a, 0x62, 0x6a, 0x72,
  ]);

  if (vdcFlags.has(modeFlag)) return ["VDC", "DC Voltage"];
  if (vacFlags.has(modeFlag)) return ["VAC", "AC Voltage"];
  if (vdcAcFlags.has(modeFlag)) return ["VDC+AC", "AC+DC Voltage"];
  if (adcFlags.has(modeFlag)) return ["ADC", "DC Current"];
  if (aacFlags.has(modeFlag)) return ["AAC", "AC Current"];
  if (adcAcFlags.has(modeFlag)) return ["ADC+AC", "AC+DC Current"];
  if (resistanceFlags.has(modeFlag)) return ["RES", "Resistance"];
  if (modeFlag === 0x03) return ["CAP", "Capacitance"];
  if (modeFlag === 0x05) return ["FREQ", "Frequency"];
  if (modeFlag === 0x45) return ["TEMP", "Temperature"];
  if (modeFlag === 0x04) return ["DIODE", "Diode"];
  if (modeFlag === 0x44) return ["CONT", "Continuity"];

  const hex = modeFlag.toString(16).padStart(2, "0");
  return [`0x${hex}`, "Unknown"];
}

/** Format a numeric result according to (unit, decimals) the way the device displays it. */
function formatResult(result, unit, decimals) {
  if (STRING_UNITS.has(unit)) {
    return result.toFixed(decimals);
  }
  // Round to `decimals` places, returned as a number.
  const factor = Math.pow(10, decimals);
  return Math.round(result * factor) / factor;
}

// ---------------------------------------------------------------------------
// Main parser
// ---------------------------------------------------------------------------

/**
 * Parse a DM40C measurement notification given as a hex string.
 *
 * @param {string} hexString - raw notification payload as hex (e.g. "df050309...")
 * @returns {{
 *   value: (string|number|null),
 *   unit: string,
 *   mode: string,
 *   modeKind: string,
 *   modeFlag: number,
 *   counts: number,
 *   overload: boolean,
 *   status: { batteryLevel: number, charging: boolean, locked: boolean, hold: boolean },
 *   raw: number[]
 * }}
 */
function parseMeasurement(hexString) {
  const payload = hexToBytes(hexString);
  const frame = extractMeasurementFrame(payload);

  if (frame === null) {
    return { value: null, unit: "", mode: "", modeKind: "", error: "No measurement frame found" };
  }

  const modeFlag = frame[5];
  const [modeKind, mode] = decodeMode(modeFlag);

  const statusByte = frame[6];
  const status = {
    batteryLevel: statusByte & 0x07,
    charging: !!(statusByte & 0x08),
    locked: !!(statusByte & 0x40),
    hold: !!(statusByte & 0x80),
  };

  const n = frame.length;
  const signFlag = frame[n - 8];
  const scaleFlag = signFlag & 0xfe;
  const auxSignFlag = frame[n - 7];
  const auxScaleFlag = auxSignFlag & 0xfe;
  const sign = signFlag & 0x01 ? -1 : 1;
  const auxSign = auxSignFlag & 0x01 ? -1 : 1;

  // M1 counts: bytes 14 (lo), 15 (hi)
  const counts = (frame[15] << 8) | frame[14];

  if (counts === 0xffff) {
    return {
      value: "OL",
      unit: "",
      mode,
      modeKind,
      modeFlag,
      counts,
      overload: true,
      status,
      raw: frame,
    };
  }

  let scaleInfo = null;
  let effectiveSign = sign;
  let effectiveCounts = DEVICE_COUNTS;

  if (modeKind === "VDC") {
    effectiveSign = auxSignFlag !== 0xff ? auxSign : sign;
    scaleInfo = VDC_MODE_SCALE_MAP[modeFlag] || null;
    if (scaleInfo === null) {
      scaleInfo = VDC_SCALE_OVERRIDE_MAP[scaleFlag] || ALT_SCALE_MAP[scaleFlag] || null;
    }
  } else if (modeKind === "VAC" || modeKind === "VDC+AC" || modeKind === "DIODE") {
    if (modeKind === "VDC+AC") {
      effectiveSign = auxSignFlag !== 0xff ? auxSign : sign;
    }
    if (
      (modeKind === "VAC" || modeKind === "VDC+AC") &&
      [0x68, 0x70, 0xa8, 0xb0].includes(modeFlag)
    ) {
      scaleInfo = ALT_SCALE_MAP[auxScaleFlag] || ALT_SCALE_MAP[scaleFlag] || null;
    } else {
      scaleInfo = ALT_SCALE_MAP[scaleFlag] || null;
    }
  } else if (["ADC", "AAC", "ADC+AC"].includes(modeKind)) {
    effectiveSign = auxSignFlag !== 0xff ? auxSign : sign;
    scaleInfo = AMP_MODE_SCALE_MAP[modeFlag] || null;
    if ([0x29, 0x69, 0xa9].includes(modeFlag)) {
      const auxScaleInfo = AMP_SCALE_MAP[auxScaleFlag];
      if (auxScaleInfo !== undefined) scaleInfo = auxScaleInfo;
    }
    if (scaleInfo === null && [0x31, 0x39, 0x71, 0x79, 0xb1, 0xb9].includes(modeFlag)) {
      scaleInfo = AMP_SCALE_MAP[auxScaleFlag] || AMP_SCALE_MAP[scaleFlag] || null;
    } else if (scaleInfo === null) {
      scaleInfo = AMP_SCALE_MAP[scaleFlag] || null;
    }
  } else if (modeKind === "CONT") {
    scaleInfo = [600.0, "Ω", 1.0, 2];
  } else if (modeKind === "RES") {
    if ([0x32, 0x72].includes(modeFlag)) {
      scaleInfo = RES_SCALE_MAP[auxScaleFlag] || RES_SCALE_MAP[scaleFlag] || null;
    } else {
      scaleInfo = RES_MODE_SCALE_MAP[modeFlag] || RES_SCALE_MAP[scaleFlag] || null;
    }
  } else if (modeKind === "CAP") {
    if (modeFlag === 0x03) {
      scaleInfo = CAP_SCALE_MAP[auxScaleFlag] || CAP_SCALE_MAP[scaleFlag] || null;
    } else {
      scaleInfo = CAP_SCALE_MAP[scaleFlag] || null;
    }
  } else if (modeKind === "FREQ") {
    if (modeFlag === 0x05) {
      scaleInfo = FREQ_SCALE_MAP[auxScaleFlag] || FREQ_SCALE_MAP[scaleFlag] || null;
    } else {
      scaleInfo = FREQ_SCALE_MAP[scaleFlag] || null;
    }
  } else if (modeKind === "TEMP") {
    scaleInfo = [6000.0, "°C", 1.0, 1];
  }

  if (scaleInfo === null) {
    // Fallback: use raw multiplier so data always shows.
    const mult = FALLBACK_MULT[scaleFlag] !== undefined ? FALLBACK_MULT[scaleFlag] : 1.0;
    const unit = FALLBACK_UNIT[modeKind] || "";
    const result = effectiveSign * counts * mult;
    return {
      value: Math.round(result * 1000) / 1000,
      unit,
      mode,
      modeKind,
      modeFlag,
      counts,
      overload: false,
      status,
      warning: `Unknown scale_flag 0x${scaleFlag.toString(16).padStart(2, "0")} for ${modeKind}, used fallback mult=${mult}`,
      raw: frame,
    };
  }

  const [fullScale, unit, unitMul, decimals] = scaleInfo;

  if (modeKind === "CAP") {
    effectiveCounts = DEVICE_COUNTS / 10.0;
  } else if (modeKind === "CONT") {
    let adjustedCounts = counts;
    if (CONTINUITY_ALT_AUX_SCALE_FLAGS.has(auxScaleFlag)) {
      adjustedCounts += CONTINUITY_WRAP_OFFSET;
    }
    const result = effectiveSign * adjustedCounts * (fullScale / DEVICE_COUNTS) * unitMul;
    return {
      value: formatResult(result, unit, decimals),
      unit,
      mode,
      modeKind,
      modeFlag,
      counts: adjustedCounts,
      overload: false,
      status,
      raw: frame,
    };
  }

  const result = effectiveSign * counts * (fullScale / effectiveCounts) * unitMul;

  return {
    value: formatResult(result, unit, decimals),
    unit,
    mode,
    modeKind,
    modeFlag,
    counts,
    overload: false,
    status,
    raw: frame,
  };
}

// ---------------------------------------------------------------------------
// Expose only parseMeasurement; helpers above stay private to the closure.
// ---------------------------------------------------------------------------

return parseMeasurement;

})();
