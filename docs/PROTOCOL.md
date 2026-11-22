# Liene Pearl N200 Pro Bluetooth protocol

Recovered from the official **Liene Photo** app:

- Android APK `com.hannto.fennel.overseas` 2.6.020 (Flutter + the legacy
  `com.hannto.avocado.lib.bt` Java SDK).
- macOS build `LienePhoto_mac.dmg` (Flutter; the same app with a Dart-side
  device kit and an IOBluetooth/CoreBluetooth native core).

The printer identifies itself in the app's `pluginInfo.json` as:

| Field | Value |
| --- | --- |
| `model` / `bundleName` | `hannto.printer.cumin` |
| `deviceName` | Liene Pearl N200 Pro |
| `pid` | `3015` |
| `protocolType` | `1` (hyphenated wire method names) |
| `channelType` | `2` (Bluetooth transport) |
| `connectType` | `8` |
| media size | `5011` (2x3 in) |
| media type | `2010` (photo paper) |
| status props | `printer-state`, `printer-sub-state`, `printer-state-alerts`, `media-size`, `battery` |

## Transport

Classic Bluetooth **Serial Port Profile (SPP)**:

- SPP service UUID `00001101-0000-1000-8000-00805f9b34fb`.
- The Android SDK bonds, then opens an insecure RFCOMM socket to that
  service UUID (`BtServiceSpp.createSocket`). It reads the RFCOMM channel
  from the SDP record.
- **The N200 Pro's SPP service is on RFCOMM channel 1.** An SDP browse
  reports three records on the live device:

  | Service | UUID | RFCOMM channel |
  | --- | --- | --- |
  | `SerialPort` | `0000 1101` (SPP) | **1** |
  | `WeChat` | `e5b152ed-6b46-09e9-4678-665e9a972cbc` | 23 |
  | unnamed | `00000000-deca-fade-deca-deaf-decacaff` | 22 |

  Channel 22 is the iAP2-style accessory channel used by the PixCut S1;
  the N200 Pro accepts Hannto frames directly on channel 1 with no MFi/iAP2
  exchange.
- On this firmware, a `performSDPQuery` filtered by a UUID array returns
  success but never invokes its completion callback. A plain SDP browse
  completes and populates `getServices()`. Opening channel 1 directly is
  the reliable path.
- For devices that need a reverse connection, the Android app first writes
  `A_OUT_<phone-bluetooth-mac>` to BLE service `0000fe95` / characteristic
  `00000013` and then acts as an SPP server. The N200 Pro path in the current
  app uses the direct SPP client flow; the BLE trigger is a fallback used
  when `needReverseConn` is set.

Data is a stream of framed packets in both directions. There is no
request/response ACK at the transport level; the printer answers JSON
requests with JSON responses carrying the same `id`.

### Optional Diffie-Hellman / encryption

The SDK contains a DH handshake (`HandShake.java`) and AES-128-ECB/RC4
encryption, but it is **disabled by default**:

- `BtConstant.NEED_HAND_SHAKE = false`
- `BtConstant.IS_HAND_SHAKE_SUCCESSED = true`
- `BtConstant.DEFAULT_ENCRYPT_TYPE = 0`

No code path in the app turns these on for `cumin`. The implementation here
therefore sends unencrypted frames. If a future firmware demands the
handshake, the frame attribute bits 10..12 carry the encryption mode and the
handshake bodies are documented by `HandShake.java` (`hello` →
server `G/P/Ra` → client `Rb/AES(P)` → `ok`).

## Frame layout

Little-endian integers throughout:

```
offset  size  field
0       1     0x7E start delimiter
1       1     version = 100
2       1     reserve = 0
3       1     connect type (1 = message/JSON, 2 = data, 0xFF = encrypted channel)
4       1     interactive (6 = request; handshake uses 16/17/18/19)
5       1     encoding (0 = UTF-8, 1 = binary, 2 = hex/binary, 3 = JSON)
6       4     terminal id (0)
10      4     message sequence number
14      2     total package count
16      2     package number (1-based for data frames)
18      2     attributes
20      N     body
20+N    1     checksum
21+N    1     0x7E end delimiter
```

`attributes`:

- bits 0..=9: body length (max 1023)
- bits 10..=12: encryption type (0 none, 1 PSK, 2 RC4, 3 RSA, 4 MiJia,
  5 AES-128-ECB, 6 AES-128-CBC)
- bit 13: split/subpackage flag

Checksum: wrapping 8-bit sum of bytes from offset 1 through the last body
byte (i.e. excluding the start delimiter, checksum, and end delimiter).

The app's parser (`FrameUtils.java`) validates the start delimiter followed
by version 100 or 1 and a zero reserve byte, then uses the 10-bit body length
to find the end delimiter.

A verified status exchange against an N200 Pro (RFCOMM channel 1, no
encryption):

```text
-> 7e 64 00 01 06 03 00000000 00000000 0000 0000 8700
   {"id":1,"method":"get-prop","params":["printer-state",
    "printer-sub-state","printer-state-alerts","media-size","battery"],
    "spec-type":1} 6d 7e

<- 7e 64 00 01 07 03 00000000 00000000 0100 0100 2c00
   {"result":["20","3015","::0",{},"4"],"id":1} cd 7e
```

The response uses interactive type 7 (response).`

## JSON messages

Envelope (serialized by `PrintJobEvent`):

```json
{
  "id": 1,
  "method": "get-prop",
  "params": ["printer-state", "printer-sub-state", "media-size"],
  "spec-type": 1
}
```

`id` is a small incrementing integer chosen by the client. Responses echo it:

```json
{ "id": 1, "result": { "...": "..." } }
```

or

```json
{ "id": 1, "error": { "code": -5001, "message": "..." } }
```

### Method names

`protocolType == 1` devices rewrite the legacy `_` in method names to `-`
(`AbstractC0702y.f`). The N200 Pro therefore uses:

| Purpose | Method | Params |
| --- | --- | --- |
| Query properties/status | `get-prop` | array of property names |
| Create print job | `print-job` | `PrintJobParams` object |
| Cancel job | `cancel-job` | `[jobId]` |
| Job info | `get-job-info` | `{"job-id": id}` |
| Job id list | `get-job-id-list` / `job-id-list` | none |
| Resume after paper/ribbon error | `resume-printer` | `[]` |
| Combo print+cut (not used by N200) | `combo-job` | array of nested jobs |

Older `protocolType == 2` devices instead use `print_job`, `confirm_job`,
`device_status`, `get_prop`, `job_info`, `resume_printer`.

### Print job

`print-job` params for a normal photo (`DeviceComboJobBean` built in
`C0/o.java` for CUMIN):

```json
{
  "copies": 1,
  "document-name": "photo.jpg",
  "file-size": 123456,
  "media-size": 5011,
  "media-type": 2010,
  "channel": 14864,
  "document-format": 9,
  "job-type": 0
}
```

- `channel` is `14864` for Perilla/Laurel/Cumin (`com.hannto.common.n.o`).
- `document-format`: 9 = JPEG, 10 = PNG, 18 = PLT.
- `job-type`: 0 = photo print; 600 = print+cut; 700 = cut-only;
  100 = firmware update.
- The app compresses the photo to a JPEG no larger than 1023 KiB and caps
  the long edge at 4096 px before sending.

The response contains the printer-assigned job id:

```json
{ "id": 2, "result": { "job-id": 20 } }
```

Firmware variants have been seen returning `job_id` instead; both are
accepted.

### Image upload

After the job is created, the client sends the file as a sequence of data
frames:

- Chunk size: **892 bytes** of file data (`RequestController`).
- Each frame body: 4-byte little-endian job id followed by the chunk.
- Header: connect type 2, interactive 6, encoding 2, version 100,
  terminal id 0.
- `msg_sn` = 1, 2, 3, ...
- `pkg_total` = `ceil(file_size / 892)`.
- `pkg_num` = 1-based chunk index.
- Attributes = 4 + chunk length (no encryption, no split).
- Full frame size is therefore 918 bytes except the last one.

For a 507,905-byte file the app sends 570 frames/3256 acknowledged writes
(observed on the sibling PixCut S1 hardware).

**There is no `confirm-job` step for protocol type 1 devices.** The completion
callback (`C0695q.onTransferCompleted`) only reports progress; the printer
starts printing when the last frame arrives. (Older devices use
`confirm_job` with `[jobId]`.)

### Status and job state

Status is read with `get-prop`:

```json
{ "id": 3, "method": "get-prop",
  "params": ["printer-state", "printer-sub-state", "printer-state-alerts",
             "media-size", "battery"],
  "spec-type": 1 }
```

Observed state categories (`DeviceStatusBean` / `PrinterParameter`):

- `printer-state`: 10 init, 20 idle, 30 sleep, 40 processing, 50 off,
  60 error.
- `printer-sub-state`: 1000 init, 2000 idle, 3002 waiting, 3015
  initialization, 3016 decoding, 30xx printing/cool-down phases, 9000
  finishing.
- `printer-state-alerts` is `::<code>`; e.g. `::5402` paper empty,
  `::5403` paper jam/error, `::8101` ribbon empty. The app's
  `error-handle-parameter` maps these to `resume-printer`.
- `battery` is a **0–4 level**, not a percentage: 0 empty, 1 low,
  2 half, 3 75%, 4 full/near-full. The status example above returns
  `"4"`, meaning the battery is full — not 4%.

Job state via `get-job-info`: `{"job-id": <id>}`. The numeric `job-state`
maps as follows (`com.hannto.common.n.Q`):

| `job-state` | Meaning |
| --- | --- |
| 3 | processing/printing |
| 7 | aborted |
| 8 | canceled |
| 9 | finished |

A successful 2x3 job observed on hardware: `job-state=3` with
`job-sub-state` 3004 → 3005 and `printing-page-number` 0 → 1, then
`job-state=9`, `job-state-reason=90001`, `print-time≈68.6 s`,
`transfer-size` equal to `file-size`.

A known idle response is `printer-state=20`, `printer-sub-state=2000`,
alerts `::0`.

### Flow control

IOBluetooth's write completion only means the local controller accepted the
bytes. The N200 Pro buffers roughly **47 frames (~42 KB)**; sending a 49 KB
job faster than it drains fills the buffer and the job aborts (`job-state=7`,
`transfer-size = 47 × 892`).

The working implementation paces **one frame every 20 ms** (≈46 KB/s) and
keeps the channel open for at least 2 s after the last frame. A 49 KB job
transfers in ~1.5 s and prints in ~69 s.

## Sources in the Android APK

| File | Contents |
| --- | --- |
| `com/hannto/avocado/lib/bt/protocol/bean/HanntoPackage.java` | frame builder/parser, checksum, attributes |
| `com/hannto/avocado/lib/bt/protocol/bean/PackageBuilder.java` | default header values |
| `com/hannto/avocado/lib/bt/protocol/util/InformationModel.java` | connect/interactive/encoding/encryption constants |
| `com/hannto/avocado/lib/bt/protocol/util/CheckSum.java` | checksum |
| `com/hannto/avocado/lib/bt/protocol/bluetooth/RequestController.java` | 892-byte chunking, job-id prefix |
| `com/hannto/avocado/lib/bt/protocol/bluetooth/BluetoothHanntoConnManager.java` | read/write, BLE `A_OUT_` trigger |
| `com/hannto/avocado/lib/bt/base/BtServiceSpp.java` | SPP socket, bonding |
| `com/hannto/avocado/lib/bt/protocol/manager/HanntoConnManager.java` | JSON frame path |
| `com/hannto/avocado/lib/wlan/PrintJobEvent.java`, `PrintJobBean.java`, `DeviceComboJobBean.java` | JSON schema |
| `com/hannto/common/utils/AbstractC0702y.java` | protocolType→method-name mapping, transport selection |
| `com/hannto/common/utils/AbstractC0688j.java` | legacy print/confirm flow |
| `C0/o.java` | N200-Pro-specific print-job construction |
| `com/hannto/common/n.java` | channel `14864`, status-code mapping |

The macOS app's Dart snapshot additionally exposes the same hyphenated
vocabulary (`print-job`, `get-prop`, `get-job-info`, `job-id-list`,
`resume-printer`, `combo-job`) and the IOBluetooth RFCOMM calls used by the
native `ht_device_bt` core.
