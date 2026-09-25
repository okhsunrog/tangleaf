# 14 — How stock Notes stores pages and strokes

This document describes how stock Onyx Notes (`com.onyx.android.note`, versionCode 45326) on the BOOX
Note Air 4C (screen 1860 × 2480 px, firmware 4.2) stores notebooks, pages and strokes. It covers what
units the pages use, what size and aspect they have, and what that means for Tangleaf's move from a fixed
1000 × 1400 sheet to a per-page size with new pages at 3:4.

Method: static reading of the decompiled app and bundled SDK, plus a read-only copy of the app's private
data taken from the tablet on 2026-09-25. No device was touched for this document. The copy holds the
user's own notes, so this document reports only structure, counts, sizes and formats. It contains no
titles, recognised text or other content.

Claim tags: **[code]** read in source or bytecode; **[data]** measured in the data copy; **[inferred]**
reasoned from code or data that was read; **[unknown]** not settled; **[recommendation]** a proposal for
Tangleaf (section 10 only).

| prefix  | location                                                                                                   |
| ------- | ---------------------------------------------------------------------------------------------------------- |
| `NA/`   | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/` (jadx; app `note/…`, bundled SDK `sdk/…`) |
| `RES/`  | `~/tmp_zfs/boox-notes-inspect/decoded/resources/res/`                                                      |
| `DIS6:` | `~/tmp_zfs/boox-notes-inspect/classes6-disassembly.txt` (dexdump of `classes6.dex`), by line number        |
| `DATA/` | `~/tmp_zfs/stock-notes-data/` (`extracted/` = the copy of `/data/data`, `exports/` = six exported PNGs)    |
| `WORK/` | `~/tmp_zfs/stock-notes-data/work/` (read-only database copies in `db/`, analysis scripts in `py/`)         |
| `TL/`   | `src/features/handwriting/` in this repository                                                             |

`~/tmp_zfs/reversed_onyx_notes_app/src` is a second decompilation of the same APK. Its
`PointDocumentLoaderV1` is byte-identical to the one cited here.

---

## 1. Summary

- **Page size on this tablet: 1860 × 2480 page units, 3:4, portrait.** This holds for every page of every
  notebook in the copy: 223 of 223 `NotePageModel.pageSize`, 222 of 222 `VirtualPageModel.pageSize`, and 56 of 56
  notebook default rects [data].
- **Units are the device's screen pixels, not paper millimetres and not normalised values.** The default page
  is built from the real screen size, `getShorterRealScreenSize()` × `getLongerRealScreenSize()`
  (`NA/sdk/scribble/data/RenderMeasure.java:45-63`) [code]. One page unit is one physical panel pixel when the
  view scale is 1.0 [inferred]. So the page size depends on the device that created the note.
- **The "mm" labels are nominal.** The UI shows the default size as "Default (186mm×248mm)", which is the pixel size
  divided by 10 (`NA/note/note/menu/NoteSizeMenuProvider.java:25`, `RES/values/strings.xml:667`) [code]. The
  paper presets use the same 10 units per mm: A4 = 2100 × 2970 (`NA/note/common/utils/NoteSizeUtils.java:8-33`)
  [code]. Physically the panel is about 300 ppi, so a default page is about 157.5 × 210 mm, not 186 × 248 mm
  [inferred from densityDpi 300, section 4.4].
- **Normal view scale 0.9435484 = 1755/1860 = 2340/2480.** The editor view is 1755 × 2340 px at screen offset
  (105, 140). The page is fitted into it with one uniform scale, and 3:4 is kept exactly. This matches the limit
  rect [0,0][1755,2340] recorded on the hardware [code][data].
- **Strokes: metadata in SQLite, points in a binary file.** Each notebook has its own SQLite database
  `<docId>.db`. Its `NewShapeModel` rows hold style, bounding box, transform and state. The points are in custom
  big-endian files `<pageId>#<revisionId>#points` under `/sdcard/.ksync/point/…`. They use 16 bytes per point:
  f32 x, f32 y (absolute page units), i16 packed tilt, i16 raw pressure (0..4095), i32 milliseconds since the
  stroke's first point [code].
- **The point file layout is confirmed on real files.** All 891 files under `/sdcard/.ksync/point` (pulled read-only
  on 2026-09-25) parse with the layout in §4.1 without an error: 48,903 strokes, 10,968,926 points, 177.9 MB, i.e.
  **16.2 bytes per point** including headers and the xref table; x 0..1855.7, y 0..2477.9, pressure 0..4095,
  non-zero packed tilt on 99.5 % of points, time offsets in ms up to 17,076 [data]. Script:
  `~/tmp_zfs/stock-notes-data/parse_points.py`.
- **Coordinates are never rescaled.** A page keeps the size of the device that created it. Other devices fit
  the whole page into their view [code][inferred].
- **For Tangleaf:** the proposed 1050 × 1400 is exactly the stock aspect. Stock units = Tangleaf units × 62/35
  (1.771428…), the same factor on both axes. Section 10 recommends keeping device-independent logical units at
  1050 × 1400 rather than adopting device pixels.

---

## 2. Storage layout

### 2.1 Files outside the app's private data

The files root is `<external storage>/.ksync`, i.e. `/sdcard/.ksync`, on Android 11 and later. On older
Android it is `com.onyx.android.ksync`'s external files directory
(`NA/sdk/path/KSyncFilePaths.java:24-30, 125-126, 141-142, 204-210`; `NA/sdk/path/AppPaths.java:48-54`) [code].

| what                     | path under the root                                              | source                                                                                                  |
| ------------------------ | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| point files              | `point/<docId>/<pageId>/<pageId>#<revisionId>#points`            | `NA/sdk/path/PointPaths.java:35-49`, `NA/sdk/data/point/PointDocumentUtils.java:15-17` [code]           |
| shape-metadata snapshots | `document/<docId>/shape/<pageId>#<revisionId>#<createdAtMs>.zip` | `NA/sdk/utils/ShapeDocumentUtils.java:161, 298-306` [code]                                              |
| page template JSON       | `document/<docId>/template/json/<pageId>.template_json`          | `NA/sdk/scribble/data/template/Templates.java:418`; `VirtualPageModel.contentRelativePath` [code][data] |
| point copies for export  | `document/<docId>/point/<pageId>/`                               | `NA/sdk/scribble/data/proto/doc/ShapePBDocumentUtils.java:16-25` [code]                                 |
| cloud templates          | `/storage/emulated/0/noteTemplate/cloud/<id>.jpg`                | MMKV `NOTE` cache entries [data]                                                                        |

Each shape-metadata snapshot is a zip holding a protobuf `ShapeInfoProtoList` with one message per shape. Its fields
mirror the `NewShapeModel` columns: uniqueId 1, createdAt 2, updatedAt 3, color 4, thickness 5, zorder 6,
boundingRect 7, matrixValues 8, textStyle 9, text 10, createArgs 11, shapeType 12, groupId 13, resource 14,
shapeStatus 15, revisionId 16, lineStyle 17, infoRevisionId 18, tagIdList 19, extra 20, linkIdList 21,
richText 22, fillColor 23, pointList 24, connectionBean 25, optionsRepo 26. Most of them are JSON strings
(`NA/sdk/scribble/data/proto/NoteShapeDocProto.java:40-65`) [code]. `toShapeInfoProto` does not fill
`pointList` (`NA/sdk/scribble/data/NewShapeModel.java:307`), so points exist only in the point files [code].
`NoteRecordDatabase.PBDocRecordModel` logs these saves: 1,517 rows over 55 documents and 191 page ids, all
with status 2 [data].

### 2.2 Databases in the app's private data

`ShapeDatabase.db` (user_version 50) is the notebook index [data]:

- `NoteModel`: 62 rows, 6 folders (type 0) and 56 notebooks (type 1). Status 1 means normal and 0 or 2 means removed
  (`NA/sdk/scribble/data/NoteModel.java:497-500`) [code]. The data has 49 normal and 7 removed [data]. The columns that matter here:
  - `pageNameList`: JSON array with the page ids in order. Ids come in two forms, 32 hex characters or a 36-character UUID [data].
  - `pageOriginWidth` / `pageOriginHeight` (REAL): the display-metrics screen size, not the page size.
    `UIBundle` sets it in the drawing args from `ResManager.getScreenWidth/Height()`, which is
    `DisplayMetrics.widthPixels/heightPixels` and not the "real" size (`NA/sdk/notecore/editor/UIBundle.java:127-130`,
    `NA/sdk/utils/ResManager.java:301-303`). It is copied back into `NoteModel` on every save
    (`NA/sdk/scribble/document/doc/NoteDocument.java:867-868`). No geometry code reads it. The only readers are
    the proto and copy paths [code]. Here it equals the page size (1860 × 2480) [data].
  - `notePageInfo` (JSON) has four parts:
    - `canvasExpandType`
    - `defaultPageRect`
    - `pageInfoMap{pageId: {width, height, layerList, currentLayerId, lastModifyTime}}`
    - `zoomInfo{viewportScale, viewPortWidth, viewPortHeight, viewPortPos, fitToScreen, scaleType}`
  - `noteBackground` (JSON): `bkGroundConfig{scaleType, canvasAutoExpand, applyAllPage}`, `docBKGround`, and
    `pageBKGroundMap{pageId: {type, resId, width, height}}`.
  - `deviceInfo` (JSON): `{deviceName, size{width, height}}`. It is built once, only while `size` is null, from
    `getShorterScreenSize()` / `getLongerScreenSize()`. Those are display-metrics sizes, not the real ones
    (`NA/sdk/data/note/NoteDeviceInfo.java:33-35`; `NA/sdk/utils/ResManager.java:236-238, 318-320`) [code]. It is
    informational only. No reader uses it for geometry [inferred].
  - `notePenInfo`, `strokeWidth`, `eraserWidth`, `pageCount`, `status`.
- `ResourceModel`: 7 rows. They are images (.png/.jpg, type 0) and audio (.pcm, type 2) [data].
- `NoteAccessHistory`: 54 rows. `ShapeModel` (the legacy shape table), `TemplateModel` and the tag, link and
  material tables are empty [data].

Each notebook has its own **`<docId>.db`**, where the docId is 32 hex characters. There are 56 files. 24 are at
user_version 22 with 38 `NewShapeModel` columns, and 32 are at user_version 24 or 25 with 40 columns (adding
`shapeConnection` and `optionsRepo`) [data]. The schema text is in `NA/sdk/scribble/data/NewShapeModel_Table.java:219` [code].

| table                           | what one row is                                                                                                                                                                                                                                  |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `NotePageModel`                 | a page: `pageSize` (RectF JSON `{left,top,right,bottom,empty,stability}`), `layerList`, `currentLayerId`, `title`, `revisionId`, `uniqueId`                                                                                                      |
| `VirtualPageModel`              | a page slot: `orderIndex` (REAL), `status` (1 = deleted), `pageSize`, `pageSizeWithMargin`, `contentPageSize` (the template's rect), `contentType` (always `"geo_layout"`), `contentRelativePath`, `contentId`, `contentPageId`, `contentStatus` |
| `VirtualDocModel`               | the notebook view: `viewportId` (the page in view), `actualScale`, `offsetX`/`offsetY`, `coverPageId`, `defaultTemplate` JSON                                                                                                                    |
| `NewShapeModel`                 | a stroke or other object; columns in section 4.4                                                                                                                                                                                                 |
| `HWRShapeModel`, `HWRDataModel` | the handwriting-recognition layer; `HWRShapeModel.points` is a BLOB in the legacy 24-byte format (section 4.6)                                                                                                                                   |
| `VirtualTocModel`               | table of contents; empty everywhere                                                                                                                                                                                                              |

Row totals over the 56 files [data]:

| table            | rows                                   |
| ---------------- | -------------------------------------- |
| NewShapeModel    | 44,460 (37,575 live, 6,885 tombstones) |
| NotePageModel    | 223                                    |
| VirtualPageModel | 222 (200 live, 22 deleted)             |
| VirtualDocModel  | 56                                     |
| HWRShapeModel    | 617                                    |
| HWRDataModel     | 617                                    |
| VirtualTocModel  | 0                                      |

A notebook has between 1 and 32 pages and between 0 and 12,731 shapes [data].

Other databases [data]:

- `NewShapeDatabase.db` (user_version 25): the same tables as a per-notebook database, all empty. It is the global
  store from older versions.
- `NoteConfigDatabase.db`: 8 key-value settings (section 7).
- `NoteStatisticsDatabase.db`: 786 usage events.
- `KSyncRecordDatabase.db`: `KNoteLocalSyncPoint` has 17 rows. `LocalRecordModel`, `ChannelRecordModel`,
  `NoteTreeLocalSyncPoint`, `SyncProgressModel` and `LocalSyncPoint` are empty.
- `filedownloader.db`: 0 downloads and 3 connection rows.

MMKV stores `files/mmkv/NOTE` (64 keys) and `HWR_work` hold the settings (section 7).
`shared_prefs/com.onyx.android.note_preferences.xml` holds only a text-input style [data].

### 2.3 What the copy lacks

The copy is `/data/data` only. None of the `/sdcard/.ksync` tree is in it (checked with `tar tf`), so it has
no point files, no shape snapshots and no template JSON [data]. All 63 `*.db-journal` files are 0 bytes, so the
databases are consistent without replaying a journal [data].

### 2.4 Layers

A shape's `zorder` is its layer id (`NA/sdk/scribble/shape/ShapeFactory.java:208`). `PageInfo` allocates layer
ids 0–4 (`NA/sdk/data/note/PageInfo.java:29`) [code]. Every page in the data has exactly one layer,
`{id 0, lock false, show true}` [data].

---

## 3. Page geometry

### 3.1 Where the page size is stored

The size is stored per page, in three places, plus a notebook default [code]:

- `NoteModel.notePageInfo.pageInfoMap[pageId].width/height`
- `NotePageModel.pageSize` and `VirtualPageModel.pageSize` (with `pageSizeWithMargin` and `contentPageSize`)
- `NoteModel.notePageInfo.defaultPageRect` for the notebook (`NoteModel.pageOriginWidth/Height` is the
  display-metrics screen size, not a page size; see 2.2)

`PageInfo.width/height` are `int` (`NA/sdk/data/note/PageInfo.java:18-20, 110`), so a page size in
`pageInfoMap` is always a whole number of units [code]. The `RectF` copies hold the same whole numbers [data].

`NotePageInfoUtils.getPageRect` falls back to the current device's default only when a stored width or height
is ≤ 0 (`NA/sdk/scribble/utils/NotePageInfoUtils.java:152-162`) [code]. A stored size therefore wins over the
device the note is opened on.

`NewShapeModel.pageOriginWidth/Height` is a separate, shape-level field that serves only legacy data (section
4.6). It is 0 in all 44,460 rows [data].

### 3.2 Unit

- `getDefaultPageWidth/Height` return the shorter and longer **real** screen size, swapped when the window is
  landscape (`RenderMeasure.java:45-47, 61-63`) [code]. On the Note Air 4C the default is 1860 × 2480 [data].
- The size menu labels treat one unit as 0.1 mm (section 3.3). The panel is 300 dpi as far as Android is concerned
  (section 4.4), so one unit is physically 0.0847 mm and the default page is about 157.5 × 210 mm [inferred].
- **No unit, DPI or physical-size field exists on pages** [data]. The only `dpi` in the data is
  `shapeCreateArgs.dpi` = 320.0. That value is a hard-coded default (`NA/sdk/data/note/ShapeCreateArgs.java:18`), not
  a measurement [code].

### 3.3 How a new page gets its size

The size menu offers these presets (`NoteSizeUtils.java:8-33`, labels in `RES/values/strings.xml`) [code]:

| id                | rect (units)                 | label                       |
| ----------------- | ---------------------------- | --------------------------- |
| `SIZE_DEFAULT`    | device default (1860 × 2480) | "Default (186mm×248mm)"     |
| `SIZE_A4`         | 2100 × 2970                  | A4 (210mm×297mm)            |
| `SIZE_A3`         | 2970 × 4190                  | A3 (297mm×419mm)            |
| `SIZE_ON_8`       | 2590 × 3680                  | 8K (259mm×368mm)            |
| `SIZE_ON_16`      | 1830 × 4190                  | 16K (183mm×419mm), as coded |
| `SIZE_ON_BIG_16`  | 2100 × 2840                  | 16K (210mm×284mm)           |
| `SIZE_ON_32`      | 1290 × 1830                  | 32K (129mm×183mm)           |
| `SIZE_ON_BIG_32`  | 1400 × 2030                  | Big 32K (140mm×203mm)       |
| `SIZE_ENVELOPE_6` | 1190 × 2290                  |                             |
| `SIZE_ENVELOPE_7` | 1610 × 2290                  |                             |

`SizeFormat` uses the same 0.1-mm convention: A4 is 2100 × 2970 and `BOUNDLESS` is 2100 × 2100
(`NA/sdk/scribble/data/bean/SizeFormat.java:15, 20`) [code].

There are two paths that decide the size [code]:

1. The menu choice goes into `NoteCreateInfo.pageRect`, which becomes `NoteDocument.defaultPageRect`
   (`NA/note/note/action/doc/CreateDocumentAction.java:120`; `NA/sdk/scribble/document/doc/NoteDocument.java:323-330`).
2. The virtual page is built from a `PaperBean` whose rect comes from the template through
   `Templates.paperTemplateRect` (`CreateDocumentAction.java:109, 141`; `NA/sdk/notecore/editor/doc/VirtualPage.java:570`).
   That function rescales the template rect to the device (`Templates.java:573-590`):
   - portrait template: height = device long side (2480), width = 2480 × template aspect. A 3:4 template gives
     1860; an A4-shaped template would give about 1753.
   - landscape template: width = device **short** side (1860), height = 1860 × template aspect. This looks odd,
     but it is what the code does.
   - no template: the device default rect (`Templates.java:424-430`).

Three special cases [code]:

- **Letterboxing.** When a page's aspect differs from the rect it is fitted to, `pageSizeWithMargin` pads it
  (`NA/sdk/notecore/editor/extension/VirtualPageSizesKt.java:35-43`).
- **Meeting scene.** The page is width × two thirds of the height (`RenderMeasure.java:65-67`).
- **Expandable pages.** `CanvasExpandType` is `DEFAULT`, `HEIGHT`, `WIDTH`, `WIDTH_HEIGHT`, `CUSTOM` or `CLIP`,
  and a page can grow to at most 2× (`NA/sdk/data/note/CanvasExpandType.java:14-16`,
  `NotePageInfoUtils.java:126-132`). The separate "draft" scene is an infinite canvas made of 2100 × 2100 cells
  (`NA/sdk/notecore/editor/config/AppConfig.java:121-124`).

How paths 1 and 2 interact for a non-default size such as A4 is [unknown]. None of the user's notebooks use one
[data].

The saved default for new notebooks is MMKV `NOTE` → `CREATE_SCRIBBLE_NOTE_ARGS_KEY`:
`pageSizeId "SIZE_DEFAULT"`, `pageSizeName "Default(186mmx248mm)"`, and background "Small Grid 2" with a
1860 × 2480 rect [data].

### 3.4 Pages of different sizes, and other devices

- **Per-page sizes are supported.** There is a rect per page in three places (3.1). `AppConfig` has a
  `resetScaleWhenGotoDiffRectPage` flag, which is true (`AppConfig.java:116, 188`), but no caller of its getter
  exists in the jadx tree [code]. The getter returns a constant, so R8 may have inlined it at its call sites. The
  missing caller does not prove the flag is unused [inferred]. The single-page editor fits each page by its own
  rect (section 5) [code]. In the data every page has the same size [data].
- **A note opened on another device** keeps its stored page size and coordinates. Nothing rescales them. The
  view fits the whole page (section 5). Background templates scale by
  `max(page.w / devDefault.w, page.h / devDefault.h)` (`NA/sdk/scribble/data/BKGroundManager.java:107-109`). The
  background `scaleType` is 0 stretch, 1 fit-centre (used everywhere here), 2 tile or 3 unscaled (`:53-60`)
  [code]. Legacy `coordType 0` shapes are the exception (section 4.6).
- **Orientation.** The default follows the window orientation at creation, so a note created in landscape gets
  2480 × 1860 pages (`RenderMeasure.java:45-63`) [code]. The data has no landscape pages [data].

### 3.5 What the data shows

Page size fields [data]:

| field                                             | result                                                                                     |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| `NotePageModel.pageSize`                          | 223 of 223 are (0, 0, 1860, 2480); every page has 1 layer                                  |
| `VirtualPageModel.pageSize`, `pageSizeWithMargin` | 222 of 222 are (0, 0, 1860, 2480); no margin anywhere                                      |
| `VirtualPageModel.contentPageSize`                | 221 are 1860 × 2480; 1 is 0 × 0                                                            |
| `VirtualDocModel.defaultTemplate.contentPageSize` | 56 of 56 are 1860 × 2480                                                                   |
| `notePageInfo.defaultPageRect`                    | 56 of 56 are {0, 0, 1860, 2480}; `canvasExpandType` is `DEFAULT` in all                    |
| `pageInfoMap` entries                             | 200 of 200 are 1860 × 2480                                                                 |
| `NoteModel.pageOriginWidth/Height`                | 55 are 1860 × 2480; one removed, empty notebook has 0 × 0 and no `zoomInfo`                |
| `noteBackground.pageBKGroundMap`                  | 55 of 55 are 1860 × 2480; `bkGroundConfig.scaleType` 1 and `canvasAutoExpand` false in all |
| `deviceInfo`                                      | all "NoteAir4C"; 51 have 1860 × 2480, and 5 (created in January 2025) have 1860 × 2354     |

The five 2354-high `deviceInfo` notebooks still have 2480-high pages. `deviceInfo` comes from display metrics,
not the real screen size (2.2) [code], so 2480 − 2354 = 126 is most likely a system bar that was counted out of
the window when those notebooks were created. It is not a page size [inferred].

Coordinates are in page units, not view pixels [data]. This was checked on the `boundingRect` of the 37,394 live
freehand strokes (shape types 2, 5, 15, 21, 22, 60, 61):

- 405 have a right edge in (1755, 1860], and 328 have a bottom edge in (2340, 2480]. Neither is possible in a
  1755 × 2340 view-pixel space.
- 13 go past x = 1860 and 19 past y = 2480. Over all live shapes the extent is x −3…2032 and y −2.5…2596. The
  outliers are text boxes and shapes pushed off the page. Saving does not clip strokes to the page.

---

## 4. Point and stroke format

### 4.1 Point file, format V1

The format is defined in `NA/sdk/data/point/PointDocument.java:20-53, 75-81, 101-119`,
`NA/sdk/data/point/v1/PointDocumentLoaderV1.java:37-143` and `NA/sdk/data/point/PointConstant.java:5-14` [code].
Everything is Java big-endian (`ByteBuffer`, `DataInputStream`), with no compression.

| offset        | size       | field                                                                                                         |
| ------------- | ---------- | ------------------------------------------------------------------------------------------------------------- |
| 0             | i16        | flag, always 0 (`PointDocument.c = 0`)                                                                        |
| 2             | i16        | version = 1; any other value throws "error version" (`PointDocument.java:23, 33-41`)                          |
| 4             | 36 B       | pageUniqueId, ASCII, right-padded with spaces to 36 (a 32-hex id gets 4 spaces; `PointDocument.java:101-111`) |
| 40            | 36 B       | revisionId (UUID)                                                                                             |
| 76            | per shape  | shape block: i16 attrA, i16 attrB, then N × 16-byte points                                                    |
| xref          | 44 B/shape | 36 B shapeUniqueId (UTF-8), i32 absolute offset of the block's attrA, i32 block length = 4 + 16·N             |
| file size − 4 | i32        | trailer: offset of the xref table                                                                             |

A point is 16 bytes:

| offset | type | field                                                                                                                                           |
| -----: | ---- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
|      0 | f32  | x, absolute page units                                                                                                                          |
|      4 | f32  | y, absolute page units                                                                                                                          |
|      8 | i16  | "size" slot, which actually holds packed tilt `(tiltX & 0xFF) \| (tiltY << 8)`, each a signed byte (`NA/sdk/data/note/TouchPoint.java:227-243`) |
|     10 | i16  | pressure, `(short) touchPoint.pressure`                                                                                                         |
|     12 | i32  | time, `timestamp − firstPoint.timestamp` (`TouchPoint.java:240-243`; `NA/sdk/pen/data/TouchPointList.java:182-192`)                             |

Other details [code]:

- attrA and attrB are always 0. Nothing outside `data/point` sets them.
- A parser can check `(fileSize − xref − 4) % 44 == 0`.
- The reader down-samples very long strokes, for rendering only. The step is `max(1, round(N / 20000))`, so it
  first takes effect at 30,000 points (`PointDocumentUtils.java:11-13`, `PointDocumentLoaderV1.java:63-70`).
- The file name is `<pageId>#<revisionId>#points` (`PointDocumentUtils.java:15-17`).

**The i16 order is tilt first, then pressure, on both write and read [code].** The writer emits `getSize()`, then
`getPressure()` (`PointDocumentLoaderV1.java:119-120`). jadx renders the reader as
`new TinyPoint(readFloat, readFloat, readShort, readShort, readInt)` (`:105-107`). The jadx constructor
stores its third argument in `d` (pressure) and its fourth in `c` (size) (`NA/sdk/data/point/TinyPoint.java:111-117`),
so the jadx reader appears to swap the two shorts. The bytecode says it does not:

- `DIS6:1115661-1115663`: the first `readShort` goes to v4 and the second to v3.
- `DIS6:1115668`: `invoke-direct/range {v0..v5}` passes v3 (the second short) as the third argument and v4
  (the first short) as the fourth.
- `DIS6:194456-194457`: the constructor stores v4 into field `c` (size) and v3 into field `d` (pressure).

The jadx constructor is right. The bug is in how jadx inlines the reader: it writes the two `readShort()` calls
in source order, while the bytecode passes the first one as the fourth argument.

### 4.2 Point units and precision

- **x, y**: f32 in page units (0…1860, 0…2480 on this device). At these magnitudes the f32 step is at most
  2⁻¹² ≈ 0.00024 units, about 0.02 µm, so storage precision is no concern [inferred].
- **pressure**: 16-bit signed field. `createArgs.maxPressure` is 4095 in all 44,460 rows [data], and it equals
  `EpdController.MAX_TOUCH_PRESSURE` (`ShapeCreateArgs.java:15`) [code]. The `(short)` cast would zero any
  normalised 0…1 value, so the stored value is the raw 0…4095 level [inferred]. The renderers divide the loaded
  pressure by `createArgs.maxPressure` (`NA/sdk/pen/NeoCharcoalPenWrapper.java:58`,
  `NA/sdk/scribble/shape/charcoal/CharcoalRender.java:100`), which fits raw storage [code]. It is unconfirmed
  without a real file [unknown].
- **tilt**: one signed byte per axis, in degrees [code for the packing; unit inferred].
- **time**: i32 relative to the stroke's first point, in milliseconds [inferred from `TouchPoint` timestamps].
  Absolute time exists only as `NewShapeModel.createdAt/updatedAt` in epoch ms [data].
- **Not stored**: the real touch size (its slot carries the tilt), and any per-point width.

The storage cost is 16 bytes per point, plus 48 bytes per shape (4-byte block header and 44-byte xref entry),
plus 80 bytes per file [code].

### 4.3 Revisions and file growth

- Each save writes a new file `<pageId>#<newRev>#points`. It contains only the newly added shapes whose
  `revisionId` is still empty (`NA/sdk/scribble/utils/ShapeSaveUtils.java:38-69`; `NotePageSaveKt.java:60-80`)
  [code].
- A shape row's `revisionId` names the file that holds its points [code].
- Moving or scaling a shape changes only `matrixValues`; the points are not rewritten [code].
- On load, every `*#points` file in the page directory is read into a map keyed by shapeUniqueId
  (`NA/sdk/scribble/document/page/NotePageLoad.java:105-132`) [code].
- In the data there are 746 distinct non-empty shape revisionIds. No revision spans two pages, so that is 746
  point files. 595 of them still hold at least one live shape. A file holds a median of 27 shapes and at most 623.
  The 2,187 shapes with an empty revisionId are all tombstones [data].
- Whether old files are ever compacted is [unknown].

### 4.4 Stroke metadata (`NewShapeModel`)

The mapping between shapes and model rows is in `ShapeFactory.java:146-236` [code].

- **Identity and state**:
  - `shapeUniqueId` (a 36-character UUID in all rows), `documentUniqueId`, `pageUniqueId`, `groupId`.
  - `createdAt` / `updatedAt` in epoch ms.
  - `status`: 0 live, 1 tombstone (`ShapeFactory.java:462-467`).
  - `revisionId` names the point file and `infoRevisionId` the metadata snapshot.
  - `syncStatus` is 0 in every row [data].
- **Type**: over all 44,460 rows, tombstones included, the counts are brush 5 (42,309), Latin calligraphy 60
  (1,090), neo brush 21 (417), Asian calligraphy 61 (315), universal 40 (178), pencil 2 (51), text 16 (36), fill 37
  (35), marker 15 (19), audio 23 (4), charcoal 22 (3), image 19 (3). Live shapes only: 5 (36,077), 60 (613), 21
  (374), 61 (262), 40 (151), 2 (47), 15 (19), 16 (18), 37 (9), 19 (3), 22 (2). The names come from
  `ShapeFactory.java:89-139` [data][code].
- **Colour**: `color` / `fillColor`, signed ARGB int; −16777216 is opaque black [data].
- **Width**: `thickness` REAL, in page units. The pen setting is in mm and is converted by
  `mmToPx(mm) = mm × densityDpi / 25.4` (`NA/sdk/extension/DimensKt.java:11-13`) [code]. The stored values are exact
  multiples of 300/25.4 [data], so densityDpi is 300 on this tablet [inferred]:

  | stored  | mm   | live shapes |
  | ------- | ---- | ----------- |
  | 7.0866  | 0.60 | 24,098      |
  | 8.2677  | 0.70 | 6,947       |
  | 7.6772  | 0.65 | 1,783       |
  | 4.7244  | 0.40 | 939         |
  | 2.9528  | 0.25 | 689         |
  | 5.9055  | 0.50 | 650         |
  | 10.0394 | 0.85 | 540         |

  Older values such as 7.2 (1,264), 11.4 and 13.8 come from the earliest notebooks [data]. The stored width is
  not divided by the view scale [data: 7.0866, not 7.0866 / 0.9435].

- **Geometry**:
  - `boundingRect`: RectF JSON. It is the bbox of the points, expanded by thickness/2 and then mapped by the
    shape matrix (`NA/sdk/scribble/shape/BaseShape.java:1187-1196`) [code].
  - `matrixValues`: `{"empty":false,"values":[9 floats]}` in Android `Matrix` order
    `[scaleX, skewX, transX, skewY, scaleY, transY, persp0, persp1, persp2]`.
  - `orientation`, `rotationPointXCoordinate`, `rotationPointYCoordinate`, `layoutType` (0 = free), `coordType`,
    `pointSaveType`, and `points` (a BLOB, NULL everywhere).
- **Creation arguments**: `shapeCreateArgs` is JSON (`ShapeCreateArgs.java`). Its fields:
  - `displayScale` / `revisedDisplayScale`: the view scale when the stroke was drawn
    (`NA/note/note/handler/scribble/ScribbleHandler.java:342`) [code]. The values are 0.9435 for 32,441 live
    shapes, 1.0 for 4,056, and zoomed values from 0.907 to 8.66 for the rest [data].
  - `maxPressure`: 4095.
  - Pen-engine parameters: `alphaFactor`, `smoothLevel` (0.2 or 0.6), `pressureSensitivity` (0.3 or 1.0),
    `newBrushRatio`, `tiltX/tiltY`, optional `penAttrs` and `tiltConfig{tiltEnabled, tiltScale}`.
  - `dpi`: 320.0, only in newer rows (section 8).
- **Style and extras**: `shapeLineStyle` JSON `{lineStyle{type, phase}}`, `textStyle`, `richText`,
  `extraAttributes`, `optionsRepo`, `resourceId`, `tagId`, `linkIdList`, `shapeConnection`.

At render time the width is `min(thickness × displayScale, mmToPx(maxWidth) × displayScale)`
(`NA/sdk/scribble/utils/ShapeUtils.java:1153-1155`) [code]. How that combines with the view matrix, and so what
on-screen width a 0.6-mm stroke has at scale 0.9435, is not traced [unknown].

### 4.5 Transforms

2,569 shapes carry a non-null `matrixValues` (2,363 live, 206 tombstones): 1,860 are translate-only, 706 have
scale or rotation, and 3 are identity [data]. These are shapes moved or resized with the lasso after they were drawn. The matrix is
applied at render time on top of the stored points, so a reader must apply it to get page positions [code].
`boundingRect` already includes it [code].

### 4.6 Legacy formats

- `pointSaveType`: 1 means `SAVE_ON_FILE`, 0 means `SAVE_ON_SQLITE` (`NA/sdk/data/PointSaveType.java:5-6`) [code].
  All 44,460 rows are 1, and `points` is NULL in all of them [data].
- **The legacy BLOB is 24 bytes per point**, big-endian: f32 x, f32 y, f32 pressure, f32 size, i64 absolute timestamp
  in ms (`NA/sdk/scribble/utils/SerializationUtils.java:14-26, 89-104`). A "WithTilt" variant packs tilt into
  the size float (`:44, :123`) [code]. The only rows in this format are the 617 `HWRShapeModel` blobs. Each is
  48 bytes, i.e. 2 points: the top-left and bottom-right corners of a recognised text box, with pressure, size
  and time all 0 [data]. The corners are in page units. Where a box's `groupId` matches live strokes, its top-left
  corner lies 16–19 units above and to the left of the strokes' union `boundingRect`, most often 17. 46 of the 617
  boxes reach past the page edge, and 13 start at (0, 0) [data].
- `coordType`:
  - 1: absolute page units. All 44,460 rows are 1 [data].
  - 0: legacy. Points are normalised to the device's longer screen side and multiplied by the **current**
    device's longer side on render (`NewShapeModel.java:83-90`) [code].
  - `ensureMigrateAbsPoints` converts 0 to 1 by baking the scale into the shape matrix
    (`NewShapeModel.java:135-151`; `ShapeUtils.java:1146-1150`). For an old note this means that the device
    that opens it fixes its absolute scale [code][inferred].

### 4.7 Partial erase

This is covered in detail in [13](13-eraser-geometry.md). In short:

- The pixel eraser drops the stored points that it hits, and each surviving run becomes a new shape
  (`NA/sdk/scribble/shape/utils/PenShapeSplitter.java:55-100`) [code].
- Runs of one point or fewer are discarded (`NA/sdk/scribble/utils/ShapeCopyUtils.java:81-89`). No points are
  interpolated at the cuts [code].
- Each piece is a clone with a new id and the same colour, width, layer, matrix, createArgs and timestamps
  (`ShapeFactory.java:488-514`) [code].
- The original row becomes a tombstone with empty points (`ShapeFactory.java:462-467`;
  `ShapeSaveUtils.java:71-83`) [code].
- The data has 23 groups of live shapes (605 shapes in all) that share one `createdAt` on the same page, which
  fits split siblings. Counting tombstones too gives 103 groups [data][inferred].

---

## 5. Scaling across devices and zoom

- **Page-to-view scale.** There are two code paths [code]:
  - **Per-page fit (the notecore editor).** A normal notebook in page mode uses `SinglePageBoxManager`
    (`NA/sdk/notecore/editor/feature/work/NormalWorkFeature.java:53`). Its minimum scale is the current page's
    own rect fitted into the viewport: `updateCurrentMinScale` → `BoxInfo.scaleToViewportWithAspectScaleCheck`
    → `RectFKt.scaleToPage` = `min(viewport.w / page.w, viewport.h / page.h)`. A page with an aspect beyond 1:5 fits
    to width or height instead (`NA/sdk/geometry/boxmanager/SinglePageBoxManager.java:386-398`, `NA/sdk/geometry/data/BoxInfo.java:501-512`,
    `NA/sdk/geometry/extension/RectFKt.java:750-763`). This box scale is the one saved as `VirtualDocModel.actualScale`
    (`NA/sdk/notecore/editor/doc/NoteDocPolicy.java:681`). It is also passed to the render context as its
    viewport scale and matrix (`NA/note/note/action/render/UpdateRenderMeasureAction.java:42-49`).
  - **Legacy `RenderMeasure` path.** `fitCanvasScale = min(devDefault.w / viewport.w, devDefault.h / viewport.h)`
    (`RenderMeasure.java:100-103`), and `renderScale = 1 / fitCanvasScale` is applied as one uniform `postScale`
    (`NA/note/note/request/render/UpdateRendererArgsRequest.java:22-33`, run from `RenderScribbleContentAction`).
    That makes `renderScale = max(viewport.w / devDefault.w, viewport.h / devDefault.h)`, which fills the view and
    can overflow it (a cover fit). It uses the device default page, not the current page.
  - On this tablet both give 1755 / 1860 = 2340 / 2480 = 0.9435484.
  - For a page that is not the default size, the per-page fit is the one that is saved and restored [code]. How
    the legacy `renderScale` combines with it on such a page is [unknown]. The fit-width, fit-height and fit-screen
    zoom actions use the page rect (`RenderMeasure.java:105-120`) [code].
- **Persisted scale.** The scale is saved as `zoomInfo.viewportScale` and `VirtualDocModel.actualScale`
  (`NA/sdk/notecore/editor/doc/NoteDocPolicy.java:681`; `RenderMeasure.java:69-82`) [code]. The data has
  [data]:
  - 51 notebooks with a 1755 × 2340 viewport at 0.943548 (3 of them with float noise such as 1754.9995).
  - 3 with a 1860 × 2480 viewport at 1.0. That is full-screen mode, with no toolbars [inferred].
  - 1 at 1.0331947 with `offsetX` ≈ 44. That is a user zoom.
  - 1 removed empty notebook with no `zoomInfo`.
- **Limit rect.** `RenderMeasure.getLimitNoteRect` is the view intersected with the page render rect
  (`RenderMeasure.java:122-128`). The notecore box manager keeps a viewport limit rect of its own
  (`NA/sdk/notecore/editor/extension/WorkBundlesKt.java:510`) [code]. On a default page either one gives the
  [0,0][1755,2340] recorded on the hardware [data][inferred]. Which of them feeds the pen limit rect is not traced
  [unknown].
- **Screen to page.** `page = (screen − (105, 140)) / 0.9435484 + pan` [inferred]. 105 : 140 is itself 3 : 4,
  so the fit leaves no letterbox on this device [inferred].
- **Zoom limits** are multiples of the fit scale (display scale = `actualScale / currentMinScale`,
  `NA/sdk/notecore/editor/extension/EditorBundlesKt.java:201-209`): normal notes 1–16×, continuous 0.5–16×, draft 0.5–2× (`NA/sdk/notecore/editor/config/ScaleConfig.java:55, 91, 121`;
  `RES/values/dimens.xml:461-462, 1242-1243, 2992-2993`) [code].
- **Other devices.** A page created elsewhere keeps its pixel size. On open, the whole page is fitted to the local
  viewport and its coordinates are not touched [code][inferred]. Pen widths are also in the creating device's
  units, since they are mm × that device's densityDpi / 25.4 [inferred]. All 56 notebooks here come from this
  tablet, so this path is not exercised by the data [data].
- **Reflowed handwriting.** It uses the same geometry. MMKV `NOTE_HWR_REFLOW_TEXT_STYLE_KEY` has `pointScale`
  0.9435483, `textWidth` 1860 and `textHeight` 2340 [data].

---

## 6. Export

- **PNG** is rendered 1:1 at the page rect (`NA/note/note/request/export/ExportToPNGRequest.java:172`, and `:223`
  for text pages) [code]. All six exported PNGs are 1860 × 2480, 8-bit RGBA, fully opaque, with sRGB and sBIT
  chunks and no `pHYs` (DPI) chunk [data].
  One export matches a stored page. Its dark-pixel box is (101, 121, 1331, 2243), and the union of that page's 459
  live shape `boundingRect`s is (101.0, 115.3, 1340.9, 2254.1). At a 0.9435 scale the right and bottom edges
  would be near 1265 and 2127, so export pixels are page units [data]. The other five exports come from pages
  that no longer exist in the current page lists [inferred].
- **PDF**, `DEFAULT` size, uses the page rect directly as the PDF page size
  (`NA/note/note/utils/NoteExportUtils.java:360-367`; `NA/sdk/note/pdf/utils/NoteEmbedUtils.java:335`) [code]. That
  makes a 1860 × 2480 pt page [inferred]. A3, A4, A5 and custom sizes are converted from mm to pt at 72 dpi and
  rounded up, e.g. A4 = 596 × 842 (`NA/sdk/utils/CommonPdfUtils.java:16`) [code].
- **Formats**: bitmap PDF, vector PDF, PNG, text, HTML and `.note` (`NA/sdk/data/note/export/ExportNoteFormat.java`)
  [code]. The exact `.note` packaging is [unknown]. The copy and export path copies point files under
  `document/<docId>/point/<pageId>/` (section 2.1) [code].
- **Sync** [code]:
  - Shapes travel as `SyncShapeModel` metadata with the same fields. On receive they are set to coordType 1 and
    pointSaveType 1 (`NA/note/note/sync/ShapeModelConverter.java:63+`).
  - Point files travel as the same per-revision files [inferred].
  - Geometry is never converted [code].
  - Sync is unused on this tablet: every shape has `syncStatus` 0, and the sync record tables are empty apart
    from 17 `KNoteLocalSyncPoint` rows [data].

---

## 7. Values found in the user's data

| item                         | value                                                                                                                    |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| notebooks / folders          | 56 (49 normal, 7 removed) / 6                                                                                            |
| pages                        | 223 `NotePageModel`, 222 `VirtualPageModel` (200 live)                                                                   |
| page size                    | 1860 × 2480 on every page; aspect 0.75                                                                                   |
| page size id                 | `SIZE_DEFAULT`, "Default(186mmx248mm)"                                                                                   |
| canvas expand                | `DEFAULT`, `canvasAutoExpand` false                                                                                      |
| layers per page              | 1                                                                                                                        |
| shapes                       | 44,460 (37,575 live); all `coordType` 1, `pointSaveType` 1, `points` NULL                                                |
| point files referenced       | 746 (595 with a live shape); median 27 shapes per file, max 623                                                          |
| view (normal)                | 1755 × 2340 at (105, 140), scale 0.9435484                                                                               |
| view (full screen)           | 1860 × 2480, scale 1.0 (3 notebooks)                                                                                     |
| `maxPressure`                | 4095 in every shape                                                                                                      |
| `shapeCreateArgs.dpi`        | 320.0 in 7,444 rows, absent in 37,016                                                                                    |
| densityDpi implied by widths | 300                                                                                                                      |
| most common width            | 7.0866 units = 0.6 mm (24,098 live shapes)                                                                               |
| default pen                  | `SHAPE_TYPE_KEY` 5 (brush), `STROKE_WIDTH_KEY` 8.267716 (0.7 mm)                                                         |
| eraser                       | `ERASER_TYPE_KEY` 1, `ERASER_WIDTH_KEY` 59.055119 (5.0 mm); `eraseWidthMap` {0: 80.314964, 1: 59.05512, 2: 5.905512}     |
| backgrounds                  | Small Grid 2 (44), Blank (4), Calendar (2), Big Grid 1 (2), Horizontal Line (2), cloud JPG (1); `scaleType` 1            |
| other MMKV settings          | `key_enable_continuous_page` 0, `ZOOM_ENABLE_KEY` 1, `FULL_SCREEN_ENABLE_KEY` 0, `INSERT_PAGE_AT_KEY` AFTER_CURRENT_PAGE |
| NoteConfigDatabase           | erase-track display off, auto PDF export off, recognition of shapes, erase and mark gestures on                          |
| exports                      | 6 PNGs, 1860 × 2480 RGBA, no DPI chunk                                                                                   |

All values are [data]. The mm figures assume densityDpi 300 [inferred].

---

## 8. Contradictions between code and data, and between sources

1. **The order of pressure and tilt in the point file.** The jadx output of the reader suggests the two i16
   fields are swapped relative to the writer. The bytecode shows they are not: tilt comes first, then pressure,
   on both paths (section 4.1). **Resolved [code].** A real file would still confirm it.
2. **`shapeCreateArgs.dpi`.** One reading of the code suggested "always 320". The data has the key in only 7,444
   of 44,460 rows, always 320.0. It first appears in March 2025, mixed with rows that lack it, and every shape
   from August 2025 on has it. In neither
   case is it the device's density (300). **Resolved [data][code].**
3. **"186 mm × 248 mm" versus the physical size.** The label is pixels / 10 (`NoteSizeMenuProvider.java:25`). The
   physical page at densityDpi 300 is about 157.5 × 210 mm. Stock's millimetre labels are nominal on this device.
   **Resolved [code][inferred].**
4. **`deviceInfo` height 2354 versus page height 2480** in 5 notebooks. `deviceInfo` is informational, and the
   pages in those notebooks are 2480 high. **Resolved [data].**
5. **Shape-level `pageOriginWidth/Height` = 0 versus notebook-level 1860 × 2480.** These are different fields.
   The shape-level one serves legacy normalised data only. **Resolved [code][data].**
6. **Two page-to-view scales.** The legacy `RenderMeasure.getFitCanvasScale` uses the device default page
   (`RenderMeasure.java:100-103`). The notecore single-page editor fits each page by its own rect and saves that
   scale as `actualScale` (section 5). For default pages the two are the same. For an A4 page they would differ.
   The saved, restored scale is the per-page one [code]. How the legacy scale combines with it on such a page is
   [unknown]. Not visible in this data.
7. **The ~1404 px page in Tangleaf's notes.** `docs/planning/handwriting-lasso-notes.md:51` and
   `TL/ink-editing.ts:134-139` assume stock's page is about 1404 px wide and scale the selection-frame constants by
   1000/1404. On this tablet the stock page is 1860 units wide, shown at 1755 screen px in the normal view
   [data]. Whether stock's selection paint sizes are in page units or screen pixels was not checked here
   [unknown]. This document changes nothing there.
8. **`SIZE_ON_16` = 1830 × 4190** (label "16K (183mm×419mm)"). The rect and label agree with each other but not with
   any usual 16K paper size. It is reported as coded [code].

---

## 9. Open questions

1. **Real point files.** Closed on 2026-09-25: every file in `/sdcard/.ksync/point` parses with the §4.1 layout
   (see the summary). Header padding and the tilt packing beyond "non-zero" were not examined further.
2. **Non-default sizes.** It is not known which path wins for A4 and other presets, or for non-3:4 templates
   (3.3), or how the view scale behaves on such a page (8.6). A test notebook created at A4, then pulled, would
   settle it.
3. **Compaction.** Whether old `*#points` files are ever merged or deleted is not known.
4. **`.note` export packaging.**
5. **On-screen width.** How `thickness × displayScale` combines with the view matrix is not traced (4.4).
6. **Screen-to-page mapping.** The formula in section 5 is inferred and could be confirmed against a hardware log
   of raw pen coordinates.
7. **Cross-device opening.** It has never been observed. All data comes from this tablet.

---

## 10. Implications for Tangleaf's page size

### 10.1 What stock chose

- **Units: screen pixels.** A stock page is the creating device's real screen in portrait, measured in physical
  pixels. It is neither a paper size nor normalised units. The paper presets exist, but they are expressed in
  those same units at a nominal 10 per mm, and the "Default" preset is simply the screen [code].
- **This device: 1860 × 2480, 3:4.** In the normal view it is shown at 0.9435 in a 1755 × 2340 area, with one
  uniform scale and no letterbox [data].
- **Per-page rects and no rescaling.** Every page stores its own rect, and stored coordinates are never
  rescaled. The single-page editor fits each page into the view by that page's own rect with one uniform
  contain scale (section 5). A different device shows the page with a different fit, and the geometry is
  unchanged [code].
- **Floats and raw values.** Coordinates are f32 floats in page units, pressure is a raw integer out of 4095,
  and widths are page units derived from mm at the device's density [code][data].

### 10.2 Tangleaf today

- The draft carries `width` / `height`, but the app hard-codes 1000 × 1400, which is 5:7 = 0.714
  (`TL/ink-model.ts:4-10, 20-21`).
- Rendering uses two independent factors, `setTransform(canvas.width / 1000, 0, 0, canvas.height / 1400, …)`
  (`TL/ink-canvas.tsx:177, 202, 221, 239, 328`). This is uniform only when the canvas is exactly 5:7.
- The ink format already has a per-page `width` / `height` as `Float` and says device DPI does not change the page
  size or canonical coordinates (`docs/planning/ink-format-v1.md` §2.3).
- Pen widths are 2, 3 and 5 units, the eraser radius is 12 units, and the grid step is 25 units, all in sheet
  units (`TL/handwriting-note-view.tsx:183, 185, 621`, `TL/ink-model.ts:115-121`).

### 10.3 Following stock versus keeping logical units

**Option A: adopt stock's device pixels (1860 × 2480 on this tablet).**

Gains:

- One unit is one panel pixel at scale 1.
- Stock data from this device could be imported with no scaling.
- Widths follow the `mm × dpi / 25.4` formula directly.

Losses:

- The page size would depend on the creating device. A page made on the desktop or on another BOOX would get
  whatever size that screen has. Stock itself shows the cost: its "mm" labels are off by 18% on this device, and a
  page from another device is fitted to the view, not kept at its physical size.
- It contradicts the ink format's rule that DPI does not change canonical coordinates.
- Every existing constant (widths, eraser radius, grid, selection frame) would need rescaling by about 1.86.

**Option B: keep device-independent logical units at 3:4 (1050 × 1400).**

Gains:

- It is the same aspect as stock and as this panel. The mapping to stock is one exact uniform factor,
  1860 / 1050 = 2480 / 1400 = 62 / 35 = 1.771428…
- Coordinates stay floats, so the unit's size costs no precision (the f32 step at 1400 is about 0.0001 units).
- The height stays 1400, the same as today's sheets.
- 1050 and 1400 are both multiples of the 25-unit grid step, giving 42 × 56 cells, so grid lines land exactly on
  the page edges.

Losses:

- One unit is not a whole number of pixels: 1.771 panel px at full width, or 1.671 px in a 1755-px-wide area.
  Grid lines drawn by the app are therefore not pixel-aligned.
- Converting to or from stock needs a multiply by 62/35 or 35/62.

**Option C: nominal 0.1-mm units, as stock's labels suggest** (for example 1575 × 2100 for the physical
157.5 × 210 mm panel, or 2100 × 2970 for A4). This gives "real" millimetres only while the page fills the panel
exactly. Stock does not keep its own mm labels true either. The gain over B is small, and every constant would
change.

### 10.4 Recommendation

- **[recommendation] New pages: 1050 × 1400 logical units (3:4).** It is exactly the stock and panel aspect, with
  an exact 62/35 relation to stock's 1860 × 2480. It keeps height 1400 and the 25-unit grid. It stays
  device-independent, as the ink format already requires. One unit is about 0.15 mm when the page spans the
  panel's full 1860-px width.
- **[recommendation] Store the size per page and never rescale existing strokes.** Old pages keep 1000 × 1400.
  This mirrors stock, which keeps each page's own rect and never touches stored coordinates when the default
  changes.
- **[recommendation] Render each page with one uniform scale**, `s = min(viewW / pageW, viewH / pageH)`, and
  letterbox the remainder. This replaces the two independent factors. It is the same formula stock's single-page
  editor uses per page (`RectFKt.scaleToPage`, section 5). Pointer mapping (`TL/ink-model.ts:20-21`) must then use
  the same `s` and offset rather than dividing by the element's width and height separately.
- **[recommendation] Keep widths, eraser radius and grid in page units** (stock keeps widths in page units too).
  For reference, on a 1050-wide page one mm at full panel width is about 6.668 units (300/25.4 × 35/62):

  | stock setting         | stock units | Tangleaf units (1050 page) |
  | --------------------- | ----------- | -------------------------- |
  | 0.25 mm               | 2.9528      | 1.667                      |
  | 0.4 mm                | 4.7244      | 2.667                      |
  | 0.5 mm                | 5.9055      | 3.334                      |
  | 0.6 mm (most used)    | 7.0866      | 4.000                      |
  | 0.7 mm (default pen)  | 8.2677      | 4.667                      |
  | eraser 5.0 mm (diam.) | 59.055      | 33.34 (radius 16.67)       |
  | eraser 6.8 mm (diam.) | 80.315      | 45.34 (radius 22.67)       |

  On a 1050-unit page, Tangleaf's current presets of 2, 3 and 5 units correspond to 0.30, 0.45 and 0.75 mm at
  full panel width, before its pressure factor [inferred]. Whether to match stock's widths is a separate decision. The table only makes
  the comparison possible.

- **[recommendation] A future stock importer** should:
  - skip tombstones (`status` 1);
  - apply `matrixValues` to the stored points;
  - scale x and y by `1050 / stockPageWidth`, one factor for both axes, and scale `thickness` by the same
    factor;
  - keep the stock page's own aspect as the imported page's size (height `1050 × H / W`), so that non-3:4
    stock pages are not distorted;
  - map pressure as `raw / shapeCreateArgs.maxPressure` (4095 in every row here);
  - keep point times as ms offsets from the stroke's first point. `createdAt` is only an approximate absolute
    anchor for that first point [inferred].

  For this tablet's pages the factor is exactly 35/62. The importer needs the `/sdcard/.ksync/point/…` files,
  which a `/data/data` copy does not contain.

- **Not recommended:** option A (device pixels) and option C (0.1-mm units), for the reasons in 10.3.

---

## Verification notes

An independent pass on 2026-09-25 re-checked this document. It used a fresh copy of the databases
(`WORK/verify/db/`) and new scripts (`WORK/verify/py/v_*.py`), and did not reuse the first pass's scripts. No
device was touched. The corrections listed below have already been made in the text above.

### Re-measured and confirmed [data]

- **Page sizes.** All of these match section 3.5:
  - `NotePageModel.pageSize`: 223 of 223 are (0, 0, 1860, 2480), each with one layer `{id 0}`.
  - `VirtualPageModel.pageSize` and `pageSizeWithMargin`: 222 of 222 are (0, 0, 1860, 2480). 200 are live and 22
    deleted. `contentPageSize` is 1860 × 2480 in 221 and 0 × 0 in 1.
  - `defaultTemplate.contentPageSize`: 56 of 56 are 1860 × 2480.
  - `defaultPageRect`: 56 of 56 are 1860 × 2480, all `DEFAULT`.
  - `pageInfoMap`: 200 of 200 are 1860 × 2480.
  - `pageBKGroundMap`: 55 of 55 are 1860 × 2480.
  - `deviceInfo`: 51 are 1860 × 2480 and 5 are 1860 × 2354, all 5 created in January 2025.
- **Row counts.** These match sections 2.2 and 7:
  - Per-notebook tables, schema versions (24 × v22/38 columns, 23 × v24 and 9 × v25 with 40 columns), and
    `ShapeDatabase` (62 `NoteModel` rows: 49 normal, 7 removed).
  - `PBDocRecordModel`: 1,517 rows, 55 documents, 191 pages, status 2.
  - The auxiliary databases, and the 63 empty journals.
- **View scale.** `zoomInfo` and `VirtualDocModel.actualScale` values match section 5: 51 notebooks at about
  0.94355, 3 at 1.0 with a 1860 × 2480 viewport, 1 at 1.0331947 with offsetX ≈ 44, and 1 without `zoomInfo`.
- **MMKV.** The `NOTE` store was parsed up to its declared data size, 22,582 bytes. Stale copies of keys lie
  beyond that size and must be ignored. The valid part has 64 distinct keys. The last valid values are
  `ERASER_WIDTH_KEY` 59.055119, `STROKE_WIDTH_KEY` 8.267716, `SHAPE_TYPE_KEY` 5 and `ERASER_TYPE_KEY` 1, as in
  section 7. `pageSizeId`/`pageSizeName` and the reflow `pointScale` 0.9435483 / `textWidth` 1860 match.
  `textHeight` is 2339.9998.
- **Shapes.**
  - All 44,460 rows have `coordType` 1, `pointSaveType` 1, NULL `points`, `syncStatus` 0 and `maxPressure` 4095.
  - `dpi` 320 is in 7,444 rows, and every shape from August 2025 has it.
  - Thickness and `displayScale` histograms match.
  - `matrixValues`: 2,569 rows set (1,860 translate, 706 scale/rotation, 3 identity).
  - The live freehand counts 37,394 / 405 / 328 / 13 / 19 and the extent x −3…2031.9, y −2.5…2595.6 all match.
- **PNG exports.** Six files, each 1860 × 2480 RGBA and opaque. Only the first export matches a live page:
  notebook `942a97…`, with a dark-pixel box within 11.1 units of the strokes' union box. The other five were
  checked against every page's union box, live or tombstone, over all 56 notebooks. None came within 95 units,
  which supports "pages that no longer exist".

### Page-unit check on other notebooks than the one cited [data]

- **Notebook `c87bf5…`.** It has 10,416 live freehand strokes without a matrix. Their point bounding box is
  `boundingRect` shrunk by thickness/2. 135 of them reach right of x = 1755, with a maximum of 1859.4, and 17 reach
  below y = 2340, with a maximum of 2451.0. Notebook `80d3e0…` gives 6 and 8. Stored points therefore span
  0…1860 × 0…2480 page units, not the 1755 × 2340 view. For untransformed strokes the right edge stays just inside 1860.
- **Notebook `71455e…`, legacy point format.** Its 105 `HWRShapeModel.points` blobs decode cleanly as big-endian
  `>ffffq`: every blob is 48 bytes, pressure, size and time are all 0, and x reaches 2240.7, y 2515.9. Decoding as
  little-endian gives values near 10³⁰, which is garbage. 79 of the boxes belong to groups that still have live
  strokes, and their corners sit a steady 17 to 18 units outside those strokes' union box. So the 24-byte decode
  and the page-unit claim both hold on a second notebook.
- **V1 point files.** The `.ksync` tree is still absent (138 tar entries, all under `databases/`, `files/` and
  `shared_prefs/`), so the 16-byte layout remains verified from code and bytecode only. The bytecode was re-read
  at `DIS6:1115655-1115668` and `DIS6:194454-194458`: tilt comes first, then pressure. The second decompilation's
  `PointDocumentLoaderV1.java` and `TinyPoint.java` are byte-identical, and both come from the same APK
  (`com.onyx.android.note`, versionCode 45326, same SHA-256).

### Corrections made

1. `NoteModel.pageOriginWidth/Height` is not the notebook's default page size. It is the display-metrics screen
   size, rewritten on every save, and no geometry code reads it (2.2, 3.1).
2. `deviceInfo` comes from display metrics and is set only once, which explains the 2354 values (2.2, 3.5).
3. The jadx pressure/tilt swap is in the inlined reader, not in the constructor (4.1).
4. Point files: 746, not "about 753" (4.3, 7).
5. The shape-type counts include tombstones, and live counts were added (4.4).
6. Split-sibling groups: 23 live groups (605 shapes), not 101. Counting tombstones gives 103 (4.7).
7. The view fit: the single-page editor fits each page by its own rect (`scaleToPage`, a contain fit) and saves
   that as `actualScale`. The device-default `RenderMeasure` scale is a legacy cover fit. Contradiction 8.6 and
   the limit-rect attribution were adjusted to match (5, 8).
8. Citations: PNG export sizing is at `ExportToPNGRequest.java:172`, and the PNGs carry an sBIT chunk (6). An
   extra `setTransform` is at `TL/ink-canvas.tsx:328` (10.2). The unused-getter argument for
   `resetScaleWhenGotoDiffRectPage` was weakened (3.4).
9. Importer: scale thickness too, divide pressure by the shape's own `maxPressure`, and treat `createdAt` only as
   an approximate time anchor (10.4).

### Recommendation check

The recommendation follows from the facts:

- 1050 × 1400 is exactly 3:4.
- 1860 / 1050 = 2480 / 1400 = 62/35.
- One unit is 0.1500 mm across the 157.48-mm panel width.
- The width table values are stock thickness × 35/62.
- 1050 and 1400 are multiples of 25.

The correction in section 5 makes the uniform-scale recommendation stronger. Stock's editor uses exactly
`min(viewW / pageW, viewH / pageH)` per page, which is the proposed formula.

### Remaining doubts

- The V1 point file layout (header padding, i16 order, raw pressure range, tilt packing, time unit) still has no
  real file behind it. It needs one `/sdcard/.ksync/point/<docId>/` directory.
- Which limit rect (legacy `RenderMeasure` or notecore box manager) the pen SDK receives, and how the legacy
  `renderScale` combines with the per-page `actualScale` on a page that is not the default size.
- The notebooks created in January 2025 have `deviceInfo` 2354 but `pageOriginHeight` 2480, even one that was
  never updated after January. The display-metrics height evidently differed between two moments of creation.
  Why is not known, and it does not affect page geometry.
- The mapping from background `type` ids (1110, 1013, 1006, 26, 4005, 4006, 0) to the template names in section 7
  was not re-derived.
