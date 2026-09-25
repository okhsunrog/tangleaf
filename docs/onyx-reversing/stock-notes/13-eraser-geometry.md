# 13 — What each stock eraser removes, and how Tangleaf differs

This document covers the model side of erasing in stock Onyx Notes (`com.onyx.android.note`, versionCode 45326)
on the BOOX Note Air 4C (firmware 4.2; system Android 13, vendor Android 11). It answers one question: given a
gesture, which strokes or parts of strokes disappear, and why. The pen-session side (render on/off, ENABLE_POST,
the post-erase pause and resume) is in [03](03-erasing.md). The hardware recording from 2026-09-25 is in
[11](11-observations.md). The second half compares each eraser with Tangleaf's `ink-editing.ts` / `ink-model.ts`.

Static analysis only. No device was touched for this document.

Claim tags: **[code]** read in source, smali or disassembly; **[inferred]** reasoned from code that was read;
**[unknown]** not settled statically.

| prefix    | location                                                                                                               |
| --------- | ---------------------------------------------------------------------------------------------------------------------- |
| `NA/`     | `~/tmp_zfs/boox-notes-inspect/decoded/sources/com/onyx/android/` (stock Notes, jadx; app `note/…`, SDK `sdk/…`)        |
| `RES/`    | `~/tmp_zfs/boox-notes-inspect/decoded/resources/res/`                                                                  |
| `SMALI7/` | `baksmali d ~/tmp_zfs/boox-notes-inspect/dex/classes7.dex` output (used where jadx names or control flow were unclear) |
| `ALG@0x…` | address in `~/tmp_zfs/onyx_framework/native/libonyx_algorithm.so`                                                      |
| `TL/`     | `src/features/handwriting/` in this repository                                                                         |
| `PLUG/`   | `plugins/tauri-plugin-mobile-system/android/src/main/java/dev/okhsunrog/mobile_system/`                                |

**jadx names.** Jadx renames getters inconsistently between classes. At the call sites used below,
`shape.getO()` is `getType()`, `getJ()` is `getStrokeWidth()`, `getL()` is `getBoundingRect()`, `getE()` is
`getLayerId()`, `getK()` is `getPoints()`, `getX()` is `getShapeMatrix()` and `getF4749d()` is
`getShapeUniqueId()`. The smali of `NotePageHitTest` shows this directly: `getLayerId` at `SMALI7/…/NotePageHitTest.smali:250`,
`getType` at `:300`, `getBoundingRect` at `:333` and `getStrokeWidth` at `:340` [code]. `getH()` / `getI()` are
stroke colour / fill colour, from the setter pairing in `ShapeFactory.shapeClone`
(`NA/sdk/scribble/shape/ShapeFactory.java:500-501`) [code].

---

## 1. Summary

- **Three erasers, three different geometries.** The stroke eraser (type 0) deletes whole shapes. The pixel
  eraser (type 1, `MOVE_ERASER`, class name "Overlay") deletes **stroke vertices** and re-creates the surviving
  runs as new shapes. The lasso eraser (type 2, `AREA_ERASER`) also deletes **vertices**, those that fall inside
  the lasso, and keeps the outside runs as new shapes. It does not delete whole strokes that the lasso merely
  touches. UI names: `RES/values/strings.xml:2997` "Stroke Eraser", `:1700` "Pixel Eraser" (`move_eraser`),
  `:191` "Lasso Eraser" (`area_eraser`) [code].
- **Hit tests use the stroke's centreline, never its width.** Stroke width enters only the bounding-box
  pre-filters (`NA/sdk/scribble/document/page/NotePageHitTest.java:75-77`, `NA/sdk/scribble/shape/BaseShape.java:273-280`)
  [code]. Pressure is never read by any eraser hit test [code, by reading every test below].
- **Eraser radius** = eraser diameter / 2, converted from screen pixels to page units by the current zoom:
  `drawRadius = width / 2`, `normalizeScale = 1 / renderMeasure.scale`, and every request uses
  `drawRadius * normalizeScale` (`NA/note/note/handler/scribble/ScribbleHandler.java:318-323`,
  `NA/note/note/request/erase/StrokesErasingRequest.java:87`, `.../OverlayErasingRequest.java:77`) [code]. The
  diameter in px is `mmToPx(mm) = mm * densityDpi / 25.4` (`NA/sdk/extension/DimensKt.java:11-13`) [code]. The
  recorded pixel-eraser value, 80.314964 px for "5.00 mm", gives a model radius of 40.157 px at zoom 1
  [inferred arithmetic].
- **Batching.** Points are buffered for 20 ms (stroke eraser) or 30 ms (pixel and lasso eraser) on this model
  (`RAW/note_noteair4c.json`, `ScribbleHandler.java:256, 629-631`) [code]. Every stroke/pixel batch is thinned so
  that kept samples lie about one radius apart (`ErasingShapeAction.y()` → `filterByGridSpacing`) [code]. The
  last point of a batch is carried into the next one [code]. The model changes batch by batch, but the app
  draws nothing until lift (see [03 §3.3](03-erasing.md)).
- **Lasso eraser runs only at lift**, on the full SDK point list. The path is turned into an Android `Region`
  (integer grid, implicitly closed, default non-zero winding) [code for the call; closing and fill rule
  inferred from Android semantics].
- **"Do Not Erase Shapes"** (`ERASE_GRAPHICS_SWITCH_KEY`, default false) makes all three erasers and both layer
  erases skip geometric shapes (lines, polylines, polygons, circles, rectangles, triangles, trapezoids,
  rhombuses, arrows, curves, wave lines, universal shapes). It has no effect on pen strokes [code, §7].
- **At lift** the app re-renders one screen rectangle: the union of the bounding boxes of every removed and
  every added shape, each expanded by about its stroke width (more for tilt pens) and 4 page units. That
  rectangle is repainted on every
  visible page it intersects, and then `ENABLE_POST(1)` is sent [code, §6].
- **Tangleaf differs in almost every geometric rule**: it adds the rendered, pressure-scaled half-width to the
  radius. It sweeps an exact capsule between samples and cuts at the exact circle boundary with interpolated
  points. Its lasso eraser deletes whole strokes that have a vertex inside **or** any segment crossing the lasso,
  and it tests inside-ness even-odd. §9 lists every difference with an example.

---

## 2. The shared pipeline

### 2.1 From pen samples to batches

The same code runs for the eraser end with the pen tool and for the pen tip with the eraser tool (routing in
[03 §3.1, §3.3](03-erasing.md)).

1. `onBeginRawErasing` → `a(j(point))` (`ScribbleHandler.java:931-935, 239-266`). This resets the carry-over
   point (`this.f = null`, `:242`), creates a fresh `EraseContext` (`:246-250`) and an Rx
   `buffer(s(), MILLISECONDS)` over the incoming points (`:256`). It then feeds the first point (`:264`) [code].
2. Each move point → `k0` → `onNext` (`:445-452`, `:1135-1137`) [code].
3. Each buffer tick → `b()` (`:277-291`), which copies the points into the whole-gesture list and calls `p()`
   (`:568-578`) → `l()` (`:454-461`). `l()` maps the batch through the render matrix (back to screen
   coordinates [inferred]) and appends it to the pending list.
   If the previous batch is still being processed (`isErasing`), it returns and leaves the points pending for the
   next tick, so nothing is lost and batches coalesce [code].
4. `m()` (`:471-524`) detaches **all** pending points (`EraseContext.detachErasingPointList()`,
   `NA/sdk/scribble/data/bean/EraseContext.java:133-137`, no decimation in this overload). It prepends a copy
   of the previous batch's last point (`:474-477`) and remembers this batch's last point (`:491`). It picks the
   visible page box that contains the **last** point (`:492-505`; no box → the batch is dropped) and maps the
   whole batch into that page's coordinates (`:507`). Then it builds `EraseArgs` (`d()`, `:318-323`) and runs
   `ErasingShapeAction` (`:508`) [code].
5. `ErasingShapeAction.create()` first runs `y()`, which thins the batch with
   `filterByGridSpacing(points, radius)` (`NA/note/note/action/erase/ErasingShapeAction.java:213-221, 225-235`). It
   then runs `x()`, which picks `StrokesErasingRequest` / `OverlayErasingRequest` / `AreaErasingRequest` by type
   (`:191-211`) [code].
6. At lift `l0()` → `h()` runs one last `m()` on the remainder, then `q()` (`:377-387, 463-469, 588-604`). `q()`
   marks the context aborted (queued batch requests return early:
   `StrokesErasingRequest.java:221`, `OverlayErasingRequest.java:178`). It stores the **full SDK point list** as
   the screen erase list (`:594-596`) and starts `EraseFinishAction` [code].

Buffer times: `strokeEraserBufferTimeMs` for type 0, `overlayEraserBufferTimeMs` otherwise
(`ScribbleHandler.java:194-196, 629-631`). Defaults are 10 / 20 ms (`NA/sdk/note/ui/config/DeviceConfig.java:79-80`),
and `RAW/note_noteair4c.json` overrides them to **20 / 30 ms** [code]. The lasso eraser uses the 30 ms buffer,
but its batch request does nothing to the model (§5.1).

**Grid thinning (`filterByGridSpacing`).** Jadx decompiles this method wrongly (its inner `break` should leave
both loops), so the smali was read: `SMALI7/com/onyx/android/sdk/pen/extension/TouchPointListKt.smali`, method
`filterByGridSpacing` [code]. For radius `f` it keeps a map of grid cells of size `f/√2`. Each point except the
last is compared with the stored points of the 3×3 neighbouring cells. If one of them is within distance `≤ f`,
the point is dropped. Otherwise it is kept and stored. The last point of the batch is always kept. Each batch
starts with an empty map, so the carried-over first point is always kept. Kept samples therefore end up a little
more than one radius apart along a dense path. The exact spacing depends on the raw sample positions and
on the 3×3 window, which can miss some within-`f` neighbours [inferred].

### 2.2 Radius and eraser sizes

| type | UI name       | diameter range                | step    | default diameter                    | source                                                                                                                                                                                                            |
| ---- | ------------- | ----------------------------- | ------- | ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0    | Stroke Eraser | 0.50–8.00 mm                  | 0.05 mm | 0.5 mm                              | `NA/sdk/scribble/utils/NoteUnits.java:32-57`, `NA/sdk/data/EraseConstant.java:33-38`, `DeviceConfig.java:96-97, 230-233`                                                                                          |
| 1    | Pixel Eraser  | 1.00–8.00 mm                  | 0.05 mm | 5.0 mm                              | same, `DeviceConfig.java:226-229`                                                                                                                                                                                 |
| 2    | Lasso Eraser  | none (range 0..0, row hidden) | —       | 0.5 mm (sent to SF, never a radius) | `NoteUnits.java:48-57`; the diameter group `binding_20` (`RES/layout/layout_shape.xml:170-174`) is shown only when not the area eraser (`NA/note/databinding/LayoutShapeBindingImpl.java:349-356, 396, 540, 650`) |

All [code]. The popup's `<` / `>` buttons move the slider by one index, i.e. 0.05 mm
(`NA/note/note/menu/popup/ErasePopup.java:367-410`, `NoteUnits.java:32-34`), and the chosen value is stored per type
(`ErasePopup.java:530-540`, `NA/sdk/data/note/EraserArgs.java:128-137, 157-159`) [code]. The default eraser type is
0 (`EraserArgs.java:51-55`) [code].

Recorded on the tablet (call tracer, 2026-09-25; rounded in [11](11-observations.md)): stroke parameters
`(8, [80.314964, 0.5, 0.1])` with the pixel eraser at "5.00 mm", and `(8, [5.905512, 0.5, 0.1])` with the lasso
eraser. `ResumeRawDrawingRequest` sends `eraserArgs.width` as the first style-8 parameter
([03 §5](03-erasing.md)), and the model reads the same field (`ScribbleHandler.v()`, `:677-679`) [code].
80.314964 = 5.00 × 408 / 25.4 and 5.905512 = 0.5 × 300 / 25.4, so the two widths were converted at different
`densityDpi` values [inferred arithmetic]. Why they differ is **[unknown]**. It does not matter for hit testing:
the lasso width is never used as a radius (§5).

The eraser disc is a circle in page units, fixed in **screen** pixels. Zooming in by 2× halves the page area it
covers (`normalizeScale = 1/scale`) [code].

### 2.3 Which shapes can be erased

`ShapeUtils.canErase(shape, doNotEraseShapes)` (`NA/sdk/scribble/utils/ShapeUtils.java:121-129, 162-166, 266-268, 833-836`)
[code]:

- a reference shape is judged by its target shape;
- stamps (type 32) are always erasable;
- links with a target are never erasable;
- image, text, attachment and rich-text shapes are never erasable. The stamp rule runs first, so a stamp is
  erasable even though the `int` overload of `isImageShape` lists type 32 as an image (`ShapeUtils.java:525-527`);
- a geometric shape (`ShapeFactory.isGraphicalShape`: types 0, 1, 7, 8, 17, 18, 24, 25, 26, 27, 28, 31, 39, 40,
  `NA/sdk/scribble/shape/ShapeFactory.java:382-384`) is skipped when "Do Not Erase Shapes" is on;
- everything else is erasable, including all pen strokes (types 2, 3, 4, 5, 15, 21, 22, 36, 47, 60, 61).

Every eraser also restricts itself to the **current layer** (`NotePageHitTest.java:70`,
`OverlayErasingRequest.java:83`, `AreaEraseFinishedRequest.java:101`) [code]. Erase-overlay shapes stored on a
page (types 14 and 20, `ShapeFactory.java:366-372`) are never candidates (§3, §4, §5) [code]. None of the three
erasers adds such a shape to the page [code, §3–§5], so they presumably come from older documents [inferred].

---

## 3. Stroke eraser (type 0): `StrokesErasingRequest`

`execute()` = `h()` then `i()` (`NA/note/note/request/erase/StrokesErasingRequest.java:218-226`) [code].

### 3.1 Candidates and hit test

`NotePageHitTest.removeShapesByTouchPointList(page, batch, r, filterEraseOverShape=true, doNotEraseShapes, rc)`
(`NotePageHitTest.java:54-101`, called at `StrokesErasingRequest.java:183-189`; the flags come from
`ScribbleHandler.java:320`) [code]:

1. Walk the page's shapes from last to first. Keep a shape if all of these hold: it is on the current layer,
   not removed, visible, free-positioned and `canErase` (`:65-70`).
2. Erase-overlay shapes (14, 20) are skipped (`:74`).
3. Pre-filter: the batch bounding box grown by `r` must intersect the shape's bounding rect grown by
   `strokeWidth/2`. `BaseShape.updateShapeRect` has already grown that rect by `strokeWidth/2`
   (`BaseShape.java:1187-1196`), so the margin is one full stroke width in total (`NotePageHitTest.java:63-64, 75-79`).
4. Exact test: `shape.hitTestPoints(batch, r)` (`:31-37`) → `ShapeHitTests.hitTestShapePoints`
   (`NA/sdk/scribble/shape/utils/ShapeHitTests.java:94-110`). For each kept sample:
   - **disc**: `shape.hitTest(x, y, r)` → for pen strokes `BaseShape.hitTest` → `hitTestShapePoint`
     (`ShapeHitTests.java:144-176`). The sample is mapped through the inverse shape matrix, then compared with
     every stroke segment `(p[i], p[i+1])` using `Algorithm.distance(…) ≤ r`
     (`NA/sdk/scribble/utils/HitTestUtils.java:226-228`). `Algorithm.distance` is native. Its disassembly
     (`ALG@0x638-0x6b4`) is the ordinary point-to-segment distance: projection parameter clamped to [0, 1],
     degenerate segment → distance to the first end. **Strokes with fewer than 2 points never hit** (`:146-148`);
   - **crossing**: for consecutive samples, `PenShape.hitTest(x1, y1, x2, y2)`
     (`NA/sdk/scribble/shape/pen/PenShape.java:123-137`), a zero-width segment/segment intersection against every
     stroke segment (`HitTestUtils.hitTestLine`, `HitTestUtils.java:197-223`; collinear segments return false).
5. Every shape that passes is removed from the page at once (`NotePage.removeShape`,
   `NA/sdk/scribble/document/page/NotePage.java:447-463`).

The early `break` for a type-20 shape at `NotePageHitTest.java:71-72` never fires. Its test
`hitTest(RenderContext, …)` is `false` in `BaseShape` (`BaseShape.java:693-696`), and no subclass overrides it
(grep of `NA/sdk/scribble/shape`) [code].

So the region a stroke eraser covers is **the union of discs of radius `r` at the thinned samples, plus the
zero-width eraser polyline**, tested against the stroke's **centreline** [code]. With the default diameter of
0.5 mm (r ≈ 0.25 mm), the stroke eraser works almost entirely by crossing [inferred].

### 3.2 What else it removes

- Shapes found by `GetReferenceShapesRequest` for the removed ones (normal and reference lists), removed too
  (`StrokesErasingRequest.java:101-104`) [code; what a "reference shape" is was not traced, **[unknown]**].
- The handwriting-recognition result for the removed shapes (`:106`, `f()` at `:58-64`) [code].
- **Fill colour of closed shapes** (`i()`, `:111-162`): on the current layer, only closed, erasable and filled
  shapes whose bounds intersect the batch bounds are considered. For each of them, if any batch sample lies
  inside the region built from the shape's path, the fill is set to 0 through an undoable attribute command
  (`j()`, `:164-181`). The outline stays. The loop **returns after the first such shape**, so one batch clears at
  most one fill [code].
- `ShapeErases.eraseFillColor(page, doc, batch, r)` (`:92`), which only affects shapes that override
  `hitTestFillArea`: universal and reference shapes (`NA/sdk/scribble/utils/ShapeErases.java:32-75`; overrides
  found only in `ReferenceShape.java:273` and `UniversalShape.java:579`) [code].

Geometric shapes are hit on their **outline** only, for example the four edges of a rectangle
(`NA/sdk/scribble/shape/graphical/RectangleShape.java:37-44, 57-65`) or the ellipse ring of a circle
(`CircleShape.java:45-60`) [code]. An eraser inside a filled rectangle clears its fill but does not delete it
(above) [code].

---

## 4. Pixel eraser (type 1): `OverlayErasingRequest`

`execute()` = `g()` (hit test and split), then `h()` (commit to the page)
(`NA/note/note/request/erase/OverlayErasingRequest.java:176-185`) [code].

### 4.1 Candidates and hit test (`g()`, `:63-135`)

- Candidates are the page's shapes plus the fragments made by earlier batches of this gesture
  (`childrenShapes`), minus the originals already split (`:69-71`) [code].
- Filter: `canErase(shape, isDisableGraphicalShapeErase())`, not an erase shape, on the current layer, and the
  shape's bounding rect intersects the batch bounds grown by `r` (`:78-83`;
  `TouchPointListKt.calculateBoundingRect`, `NA/sdk/pen/extension/TouchPointListKt.java:21-32`) [code]. There is no
  `isVisible` / `isRemoved` / `isFreePosition` check here, unlike the stroke eraser [code].
- A sample is a **hit sample** for this shape if `fastHitTest` (a circle of radius `r` against the bounds grown by
  another `strokeWidth/2`, `BaseShape.java:273-280`) passes and then one of two things holds. Either
  `shape.hitTest(x, y, r)` is true, which for pen strokes is the same point-to-segment centreline test as in §3.1.
  Or, for a closed shape with a fill, the sample lies inside the fill region (`:85-99`) [code].
- **No crossing test**: the pixel eraser tests only sample points, not the chords between them [code].

### 4.2 How a stroke is split

With at least one hit sample, the request calls `shape.split(EraseBean(hitSamples, r))` (`:100-103`). For pen
strokes that is `PenShapeSplitter.split` (`PenShape.java:248-251`,
`NA/sdk/scribble/shape/utils/PenShapeSplitter.java:55-95`) [code]:

1. The hit samples are mapped into the stroke's local space (`EraseBean.setTargetShapeMatrix`,
   `NA/sdk/scribble/data/bean/EraseBean.java:83-93`).
2. Each stroke **vertex** `p[i]` is tested with `EraseBean.isPointHitTest(p[i-1], p[i])` (`EraseBean.java:39-61`).
   For `i = 0` the test is whether `p[0]` lies within `r` of a hit sample (`PointUtils.isPointInsideCircle`,
   `NA/sdk/utils/PointUtils.java:25-29`). For `i > 0` it is whether the **incoming segment** `p[i-1]→p[i]` passes
   within `r` of a hit sample. A vertex that passes is removed.
3. If no vertex is removed → no change. If fewer than 2 vertices remain in total → the whole shape is erased
   (`PenShapeSplitter.java:78-84`).
4. Otherwise the remaining vertices are split into runs of consecutive survivors (`a()`, `:30-52`). Each run
   becomes a new shape through `ShapeCopyUtils.cloneNewShapes`, and **runs of one vertex are dropped**
   (`NA/sdk/scribble/utils/ShapeCopyUtils.java:81-101`).

Consequences [inferred from the code above]:

- The cut happens at **vertex granularity**. Nothing is interpolated at the eraser boundary.
- Removing `p[i]` removes both segments that meet at it, so the gap in the drawing runs from `p[i-1]` to
  `p[i+1]`. Because the test looks at the **incoming** segment, a touch in the middle of `p[i-1]→p[i]` removes
  `p[i]` but not `p[i-1]`. The gap is biased toward the direction the stroke was drawn.
- **3-vertex edge case**: for `[A, B, C]` with only `B` hit, 2 vertices remain, so the shape is not "erased". The
  runs `[A]` and `[C]` are both dropped, so `splitShapes` is empty. `OverlayErasingRequest` then takes neither the
  "split" branch nor the "erased" branch (`:104-115`), and **the stroke stays unchanged** [code].

### 4.3 The new shapes

`ShapeFactory.cloneNewShape(shape, type)` → `shapeClone` + `b()` (`ShapeFactory.java:192-198, 488-551, 610-614`)
[code]:

- The fragment has the **same type**, stroke colour, fill colour, layer, stroke width, layout type, text,
  created/updated timestamps, scribble type, shape matrix, transforming/reflow matrices, persist options, text
  style, line style, create args, resource, reference/source beans, object id and custom attributes.
- It gets a **new unique id** (`ensureShapeUniqueId`, `:511`). Tags, links and universal links are cleared, and
  so are the revision ids (`b()`).
- Its points are the original `TouchPoint` objects of the run (`ShapeCopyUtils.java:85-87`), so pressure, size
  and timestamps are kept. No point is added.
- The original is marked transparent and later removed. Fragments are **appended to the end of the page's shape
  list** (`BaseEraseRequest.addShapes` → `addShapeList$default(…, i = -1)`,
  `NA/note/note/request/erase/BaseEraseRequest.java:120-129`,
  `NA/sdk/scribble/document/page/NotePageShapeKt.java:113-131, 151-156`) [code]. That places them above every
  other shape of the layer in list order [inferred: that list order is paint order].

### 4.4 Per-batch commit

`h()` (`:137-173`) adds this batch's fragments to the page, removes the split originals, and applies
`eraseFillColor` with radius `r`. If anything changed, it clears the context lists [code]. The next 30 ms batch
therefore works on the fragments, not on the original stroke [code].

### 4.5 Geometric shapes and fills

`GraphicalShape.split` → `GraphicalShapeSplitter.split(shape, bean, 36)`
(`NA/sdk/scribble/shape/graphical/GraphicalShape.java:80-83`, `NA/sdk/scribble/shape/utils/GraphicalShapeSplitter.java:114-216`)
[code]:

- The outline path is sampled **every 2 units** (`:125-161`), and every sample is tested with
  `isPointHitTest(prev, cur)`.
- Runs of surviving samples become **`FreeShape` (type 36) polylines** with an identity matrix
  (`cloneNewShapesAndRestMatrix`, `:184`; `ShapeFactory.java:108`). A partially erased rectangle therefore
  becomes freehand strokes.
- For a closed shape, the last run is joined to the first when the path end survived (`:162-164`).
- A closed shape with a fill also has its fill region converted to a `FillShape` (type 37) and split
  (`a()`, `:42-61`). If that fill split erased nothing, the method returns an **empty** result and the outline
  split is discarded too (`:193-197`) [code]. What that does on screen is **[unknown]**.

With "Do Not Erase Shapes" on, `canErase` already drops geometric shapes (§2.3). The `setFillColor(0)` branch in
`f()` (`OverlayErasingRequest.java:53-61`) is then reachable only through reference shapes [inferred].

---

## 5. Lasso eraser (type 2): `AreaErasingRequest`, `AreaEraseFinishAction`, `AreaEraseFinishedRequest`

### 5.1 During the gesture

`AreaErasingRequest.execute()` only switches the render mode and stores the erase args
(`NA/note/note/request/AreaErasingRequest.java:22-29`). The model does not change while the pen is down [code].

### 5.2 At lift: from the path to a region

- `AreaEraseFinishAction` takes the **full SDK point list** (`EraseContext.screenErasePointList`, set in
  `ScribbleHandler.q()`; `NA/note/note/action/erase/AreaEraseFinishAction.java:59-66`). No grid thinning applies
  to it [code].
- For every visible page box that intersects the list's bounds (`:69-73`), it maps the points into that page and
  builds an `AreaEraseShape` (type 20) from them, with stroke width `2 / renderScale` (`t()`, `:90-98`). It then
  runs `AreaEraseFinishedRequest` on that page (`u()`, `:100-131`) [code].
- **Region**: `AreaEraseShape.hitTest(x, y, _)` lazily builds `ShapeRegions.createShapePointRegion`
  (`NA/sdk/scribble/shape/erase/AreaEraseShape.java:33-43`, `NA/sdk/scribble/utils/ShapeRegions.java:22-29`) [code].
  The path is `ShapeRenderUtils.renderShape(null, points)`: `moveTo(p0)`, then `quadTo(mid(p[i-1], p[i]), p[i])`
  for each point (`NA/sdk/scribble/utils/ShapeRenderUtils.java:241-261`). A quadratic whose control point is the
  chord midpoint is the straight chord, so the path is the polyline through the samples [code + arithmetic]. For
  lists over 20 000 points, `getRenderPoints()` decimates them (`NA/sdk/pen/data/TouchPointList.java:111-119`) [code].
- **Closing**: the path has no `close()`. `Region.setPath(path, clip)` rasterises the filled path, and filling
  closes every contour from its last point back to its first. The fill rule is the path's default,
  `FillType.WINDING` (non-zero) [inferred from Android/Skia behaviour; not read in this pass].
- **Raster**: the region lives on the integer page-unit grid and is clipped to the lasso's bounding rect cast to
  `int`. A test point is truncated with `(int)` before `region.contains` (`AreaEraseShape.java:37-42`,
  `ShapeRegions.java:25-27`) [code]. Membership is therefore quantised to about one page unit [inferred].

### 5.3 What is removed (`AreaEraseFinishedRequest.i()`, `NA/note/note/request/erase/AreaEraseFinishedRequest.java:96-128`)

- Candidates: current layer, `canErase(shape, isDisableGraphicalShapeErase())`, not an erase shape, and the lasso
  bounding rect contains or intersects the shape's bounding rect (`:101-105`) [code].
- `shape.split(EraseBean().setEraseShape(lasso))` (`:106-108`). With no erase points, `isPointHitTest(prev, cur)`
  ignores `prev`. It maps the **vertex** `cur` through the stroke's matrix and asks whether the lasso region
  contains it (`EraseBean.java:40-50`) [code].
- For pen strokes, `PenShapeSplitter` then applies exactly the rules of §4.2: vertices inside the lasso are
  removed, outside runs of at least 2 vertices become new shapes with the attributes of §4.3, and if fewer than
  2 vertices remain the shape is erased [code].
- So the lasso eraser requires neither "whole stroke inside" nor "any point inside" at the stroke level. It
  **cuts out the inside vertices**. A stroke is untouched when none of its **vertices** is inside, even if one
  of its segments passes straight through the lasso. A stroke with every vertex inside disappears [inferred from
  the code above].
- Geometric shapes are sampled every 2 units and split into `FreeShape` runs, and filled closed shapes have their
  fill split too, as in §4.5 [code].
- The request then adds the fragments (appended to the list, `:158-159`), removes the originals and their
  relational shapes (`:163-165`) and runs `eraseFillColor(page, doc, lassoPoints, 1.0)`, which again only affects
  universal and reference shapes (`:166`). It clears the HWR results (`:169`) and unions the dirty rect (`j()`,
  `:130-137`) [code].
- The lasso's own diameter setting is never used. The radius passed to `hitTest` is the constant `1.0f`, and
  `AreaEraseShape.hitTest` ignores it [code].

---

## 6. How the result reaches the screen

- **During the gesture**: for the raw-rendered types 0, 1 and 2, `ErasingShapeAction.s()` returns at once
  (`ErasingShapeAction.java:101-109`). The app draws nothing, and SurfaceFlinger paints the trace
  ([03 §3.3](03-erasing.md)) [code].
- **At lift**: `EraseFinishAction.G()` runs the finish request for the type (`:178-223`). For types 0 and 1 that
  only resets state (`StrokesEraseFinishedRequest.java:20-24`, `OverlayEraseFinishedRequest.java:38-59`), because
  the model already changed batch by batch. For type 2 it runs the lasso pass of §5 [code].
- `EraseFinishAction.render()` (`:343-376`) takes `EraseContext.getChangedShapeList()` (removed + added + changed +
  tag + universal-link shapes, `EraseContext.java:161-177`). If that list is empty, or no changed page is visible,
  it returns without drawing [code]. Otherwise it calls `forceRenderVisibleDirtyScreenRectToDisplay`
  (`NA/note/common/extension/ForceRenderSliceListActionsKt.java:267-288`). The dirty rectangle is **one union**:
  for each changed shape, its bounding rect (already grown by `strokeWidth/2`) grown again by
  `strokeWidth/2 × matrix scale × a per-shape factor`, tripled for tilt pens
  (`NA/sdk/scribble/utils/ShapeRectUtils.java:24-39`), and by 4 page units
  (`ShapeRectUtils.getBoundingRectWithLink`, `NA/sdk/scribble/shape/utils/ShapeRectUtils.java:67-80`), mapped to
  the screen and united (`getShapeScreenRect`, `:136-156`; `RectFKt.unionOrCopy`,
  `NA/sdk/geometry/extension/RectFKt.java:867-874`). That rectangle is re-rendered in every visible page box it
  intersects, then `InkRenderManager.invalidate(enablePost=true, invalidate=false)` sends `ENABLE_POST(-1,1,pid)`
  (`:541-567`; [03 §3.4](03-erasing.md)) [code].
- The rectangle covers the **whole** removed strokes, not just the erased piece, because a split original is a
  removed shape [inferred]. The lasso outline and the eraser path are not in it [code: only shapes are
  united]. The hardware log shows one `waveform[2] update_mode[0]` partial update of the erased area after each
  erase ([11](11-observations.md)).

---

## 7. "Do Not Erase Shapes" and the other menu switches

- The popup toggle flips `ERASE_GRAPHICS_SWITCH_KEY` (`ErasePopup.java:99-115, 149-151`;
  `NA/sdk/note/ui/common/utils/NoteMMKVHelper.java:1089-1091`, default false). The row is visible only when the
  current scene supports graphics (`ErasePopup.java:168-171`, `LayoutShapeBindingImpl.java:260-263, 388, 528-531`)
  [code].
- It is read by the stroke eraser (`ScribbleHandler.java:320` → `NotePageHitTest.java:70`,
  `StrokesErasingRequest.java:122`), the pixel eraser (`OverlayErasingRequest.java:78, 83`), the lasso eraser
  (`AreaEraseFinishedRequest.java:99-101`), both layer erases (`EraseCurrentLayerAction.java:44`,
  `EraseAllLayerShapesAction.java:64`) and the HWR scratch-out gesture (`HWRHandleEraseGestureAction.java:121`)
  [code]. In every eraser it removes geometric shapes from the candidates before any hit test, so a shape is
  neither deleted, split nor stripped of its fill [code for the gate; "nor stripped of its fill" follows because
  `i()` also filters with `canErase`].
- "Auto-deselect Eraser" only switches back to the previous tool after the erase ([03 §3.4](03-erasing.md)); it
  does not change geometry [code].

---

## 8. Tangleaf today

- **Units**: every stroke lives on a 1000 × 1400 sheet. Native points are normalised as
  `x = (px − sheet.left) / sheet.width × 1000`, `y = … / sheet.height × 1400` (`PLUG/OnyxInk.kt:857-864`) [code].
  The eraser radius is fixed in sheet units: 6, 12 (default) or 24 (`TL/handwriting-note-view.tsx:184-185, 671-677`)
  [code]. `InkStroke` has only `id`, `width` and `points`: no type, colour or layer (`src/lib/bindings.ts:531-535`)
  [code].
- **Modes**: "stroke", "pixel" and "lasso" (`TL/ink-editing.ts:3`), default "stroke"
  (`TL/handwriting-note-view.tsx:184`) [code]. The eraser end with the pen tool and the pen tip with the eraser
  tool both arrive as `erasing` (`PLUG/OnyxInk.kt:771`) and use the selected mode
  (`TL/ink-canvas.tsx:337-340, 425-444`) [code].
- **When it runs**: while the pen moves, the plugin sends at most one preview point every 32 ms
  (`PLUG/OnyxInk.kt:866-875`). For stroke/pixel mode, each preview applies `eraseGesture` to the running preview
  with the segment `[last, point]` and republishes (`TL/ink-canvas.tsx:389-396`). At lift the result is
  **recomputed from the pre-gesture strokes** with the full SDK list (`TL/ink-canvas.tsx:399-412, 439-444`) [code].
- **`eraseGesture`** (`TL/ink-editing.ts:302-378`) [code]:
  - Douglas–Peucker-simplify the eraser path with tolerance 0.5 (`:309`, `:44-65`).
  - Stroke/pixel broad phase: stroke bounds against path bounds grown by `r + 1.75·w/2` (`:318`).
  - Per stroke segment `a→b`: `cutIntervals(a, b, path, R)` with `R = r + w·(0.25 + 1.5·max(pa, pb))/2`, the
    rendered half-width at that segment (`:338-343`; the render width formula is at `TL/ink-model.ts:87`).
    `cutIntervals` returns the exact parameter intervals of the segment inside the **swept capsule**: a circle at
    every sample plus a slab of width `2R` between consecutive samples (`:230-301`).
  - Stroke mode: any interval → the whole stroke is deleted (`:352, 368-375`).
  - Pixel mode: the segment is cut at the interval ends, and the cut points interpolate x, y, pressure, tilt and
    time (`:23-32, 353-365`). Consecutive duplicates are skipped (`:329-333`). The first fragment keeps the
    original `id`, the others get `crypto.randomUUID()`, all keep `width`, and they stay at the original's index
    (`:371-374`).
  - Lasso mode: `selectLasso(strokes, simplifiedPath)` and delete the selected strokes (`:310-313`).
- **`selectLasso`** (`TL/ink-editing.ts:97-115`) [code]: no-op if the polygon has fewer than 3 points or its bounds
  are narrower than 2 units on either axis. Otherwise a stroke is selected if its bounds overlap the polygon bounds
  **and** some vertex is `inside` (even-odd ray test, points within 1e-8 of an edge count as inside, `:72-82`),
  **or** some stroke segment `crosses` some polygon edge, closing edge included (`:83-96, 109-111`).
- **`eraseAt`** (`TL/ink-model.ts:40-49`) deletes whole strokes whose segments come within `12 + width/2` of a
  point. It is used only by `applyOnyxStroke` (`TL/onyx-ink.ts:26-32`), which runs only when no `onStroke`
  handler is given (`TL/onyx-ink.ts:258-262`). `ink-canvas.tsx` always passes one (`TL/ink-canvas.tsx:469`), so
  `eraseAt` is not on the live path today [code].
- **Render at lift**: `publish()` damages the union of `inkBounds` of removed and added strokes (bounds grown by
  `1.75·w/2 + 2`, `TL/ink-region.ts:20-52`), plus the previous overlay and the current overlay
  (`TL/ink-canvas.tsx:307-322`). During an erase the overlay covers every gesture point grown by `r + 2`. At lift
  `active.current` is already cleared (`:415`), so the last preview's eraser-trail rectangle enters only as the
  previous overlay [code].

---

## 9. Every behavioural difference, with an example

Unless stated otherwise, the examples use the same number for stock's page-unit radius `r` and Tangleaf's
sheet-unit radius `r`. Stroke vertices are listed explicitly, and Tangleaf strokes have width `w` and pressure
`p` on every point, so the rendered half-width is `h = w·(0.25 + 1.5p)/2`.

### 9.1 Common to all three erasers

- **C1. Size model.** Stock: a diameter in mm, converted to screen px and then to page units by the zoom (§2.2).
  Tangleaf: a radius in sheet units, whose on-screen size follows the sheet's CSS size, with no mm value.
  _Example:_ the same "5.00 mm" pixel eraser is 40.16 px in radius on stock at zoom 1 [inferred arithmetic]. In
  Tangleaf, "Medium" is 12 units, i.e. `12 × sheet.width/1000` px on screen [code].
- **C2. Anisotropy.** Tangleaf maps x by `sheet.width/1000` and y by `sheet.height/1400` separately
  (`PLUG/OnyxInk.kt:858-859`). If the sheet is not drawn at exactly 5:7, the eraser "circle" is an ellipse on
  screen [inferred]. Stock's disc is a circle in page space [code].
- **C3. Timing and batching.** Stock changes the model every 20/30 ms batch on thinned samples, and the result
  becomes visible only at lift (§2.1, §6). Tangleaf changes a preview every ≥32 ms and recomputes everything from
  the pre-gesture state at lift. _Example:_ an eraser moving along a quarter circle of radius 100 whose previews
  arrive 20 units apart. Tangleaf's preview sweeps straight chords that cut up to `100 − √(100² − 10²) ≈ 0.5`
  units inside the arc, while its final result follows the SDK points. Stock has no app preview at all [inferred].
- **C4. Layers, shape kinds, fills, HWR, references.** Stock limits erasing to the current layer, never erases
  images/text/attachments, honours "Do Not Erase Shapes", clears fills and HWR results, and removes reference
  shapes (§2.3, §3.2). Tangleaf has only untyped single-colour strokes. _Example:_ a rectangle drawn with the shape
  tool. Stock with "Do Not Erase Shapes" on leaves it alone. Tangleaf has no such object [code].
- **C5. Pressure.** Stock ignores pressure in every hit test. Tangleaf widens its radius by up to
  `1.75·w/2` at full pressure. _Example:_ two identical Tangleaf strokes at the same distance from the eraser path,
  one at p=0 and one at p=1, can get different results. Stock treats them the same [code].

### 9.2 Stroke eraser (stock type 0 ↔ Tangleaf "stroke")

- **S1. Stroke width.** Stock tests the centreline only (§3.1). Tangleaf tests against `r + h`.
  _Example:_ stroke `(0,0)…(100,0)` with a vertex every unit, `w = 4`, `p = 1` (`h = 3.5`); the eraser moves along
  `y = 14` from x = 20 to 80; `r = 12`. Stock: 14 > 12 and no crossing → **kept**. Tangleaf: 14 ≤ 15.5 →
  **deleted**.
- **S2. Coverage between samples.** Stock covers discs at the thinned samples plus the zero-width eraser chord.
  Tangleaf covers the full capsule. _Example:_ `r = 10`, consecutive kept samples `(0,0)` and `(10.5,0)` (a dense
  path thinned by §2.1), and a vertical stroke from `(5.25,40)` down to `(5.25,9)`. Stock: its nearest point is
  10.42 from both samples, and it does not reach the chord → **kept**. Tangleaf: distance to the chord is 9 ≤ 10 →
  **deleted**.
- **S3. Sparse samples with no crossing.** _Example:_ a fast swipe with consecutive samples `(0,26)` and
  `(100,26)`, `r = 8`, stroke `(50,0)–(50,20)`, `w = 2`, `p = 0.5` (`h = 1`). Stock: samples are 50.4 away and the
  chord `y = 26` does not cross the stroke → **kept**. Tangleaf: capsule distance 6 ≤ 9 → **deleted**.
- **S4. Dots.** Stock's point test needs at least 2 points (`ShapeHitTests.java:146-148`), and the crossing loop is
  empty for 1 point, so a one-point pen shape can never be hit by the stroke eraser. Tangleaf treats a one-point
  stroke as a zero-length segment (`TL/ink-editing.ts:238`). _Example:_ a stroke with the single vertex `(50,50)`,
  eraser passing through it: stock **kept**, Tangleaf **deleted**. Whether stock ever stores a one-point pen shape
  is **[unknown]**.
- **S5. Default size.** Stock's default stroke-eraser diameter is 0.5 mm (r ≈ 0.25 mm), so in practice it erases
  what the eraser path crosses. Tangleaf's default radius is 12 units plus `h` [code]. _Example:_ the eraser end
  passing 1 mm beside a stroke without touching it: stock **keeps** it at the default size, Tangleaf deletes it
  if 1 mm is under `(12 + h)` units on screen [inferred].
- **S6. Fills.** Stock clears the fill of at most one closed filled shape per batch when a sample lands inside it
  (§3.2). Tangleaf has no fills [code].

### 9.3 Pixel eraser (stock type 1 ↔ Tangleaf "pixel")

- **P1. Cut position.** Stock removes whole vertices by the incoming-segment rule and interpolates nothing
  (§4.2). Tangleaf cuts at the exact capsule boundary and interpolates. _Example:_ stroke `A(0,0) B(10,0) C(20,0) D(30,0)`,
  one eraser sample at `(5,4)`, `r = 5`, Tangleaf `w = 2`, `p = 0.5` (`R = 6`). Stock: `AB` is 4 away, so `B` goes.
  `A` (6.4 away) and `C` (segment `BC` 6.4 away) stay. The runs are `[A]` (dropped) and `[C, D]`, so **one new
  stroke `C–D`** remains, a visible gap from x = 0 to x = 20. Tangleaf: `AB` is cut on x ∈ [0.53, 9.47], leaving
  **`A–(0.53,0)`** (original id) and **`(9.47,0)–B–C–D`** (new id).
- **P2. The 3-vertex no-op.** _Example:_ stroke `A(0,0) B(10,0) C(20,0)`, same sample and radius. Stock: only `B` is
  hit, both remaining runs have one vertex, the split result is empty and "not erased" → the stroke is
  **unchanged** (§4.2). Tangleaf: **two fragments**, as in P1.
- **P3. Width.** _Example:_ Tangleaf stroke along `y = 0` with `w = 20`, `p = 1` (`h = 17.5`); the eraser moves
  along `y = 25` with `r = 10`. Stock with the same centreline: 25 > 10 → **kept**, even though the eraser disc
  overlaps the visible ink band. Tangleaf: 25 ≤ 27.5 → **cut**.
- **P4. No crossing test.** Stock's pixel eraser tests sample points only (§4.1). _Example:_ `r = 5`, consecutive
  samples `(0,0)` and `(30,0)`, and a vertical stroke `x = 15` from y = −50 to 50 with a vertex every unit. Stock:
  both samples are 15 away → **kept**. The stroke eraser in the same spot would delete it through the crossing
  test. Tangleaf: **cut** around y ∈ [−R, R].
- **P5. Fragment identity and order.** Stock gives **every** fragment a new id, drops tags and links, and appends
  the fragments at the end of the page's shape list (§4.3). Tangleaf keeps the original id on the first fragment
  and keeps fragments at the original's position. _Example:_ erase the middle of stroke S drawn under stroke T.
  Stock ends with both S fragments above T in list order. Tangleaf keeps them below T [code; visible only where
  ink overlaps with different colours, which Tangleaf does not have yet].
- **P6. Minimum fragment size.** Stock drops one-vertex runs and erases the whole shape when fewer than 2
  vertices would remain. Tangleaf keeps any fragment with at least one distinct point, and the interpolated cut
  points usually give it at least two. _Example:_ stroke `A(0,0) B(10,0)`, one sample at `(9,3)`, `r = 5`. Stock:
  `AB` is 3 away, so `B` goes, and 1 vertex remaining means the **whole stroke is erased**. Tangleaf (`R = 6`, as
  in P1): `AB` is cut on x ∈ [3.8, 10] (the circle's far edge, x = 14.2, lies past `B`), leaving
  **`A–(3.8,0)`**.
- **P7. Per-batch commit.** Stock commits fragments every 30 ms and later batches see the fragments (§4.4).
  Tangleaf's final result comes from one pass over the full path. The two give the same union only where the rules
  above agree [inferred].
- **P8. Geometric shapes.** Stock turns a partially erased geometric shape into `FreeShape` polylines sampled
  every 2 units, and splits its fill (§4.5). Tangleaf has none [code].

### 9.4 Lasso eraser (stock type 2 ↔ Tangleaf "lasso")

- **L1. Cut out versus delete.** Stock removes the vertices inside the lasso and keeps the outside runs (§5.3).
  Tangleaf deletes every selected stroke whole. _Example:_ stroke `(0,100)…(300,100)` with a vertex every 10
  units, and a lasso that is a circle of radius 30 around `(150,100)`. Stock: the vertices from x = 130 to 170 go,
  and **two strokes remain**, x = 0…120 and x = 180…300. Tangleaf: the **whole stroke is deleted**. The vertices
  at x = 120 and x = 180 lie on the circle, where stock's pixel region and Tangleaf's `inside` may disagree (L4).
- **L2. Crossing segments.** Stock tests vertices only. Tangleaf also selects strokes whose segments cross the
  lasso. _Example:_ the two-vertex line `(0,100)–(300,100)` with the same lasso. Stock: no vertex inside →
  **unchanged**. Tangleaf: the segment crosses the polygon → **deleted**.
- **L3. Fill rule.** Stock uses non-zero winding [inferred], Tangleaf even-odd [code]. _Example:_ the user draws
  the lasso twice round the same circle, and a short stroke lies entirely inside. Stock: winding number 2 →
  inside → the stroke is **erased**. Tangleaf: an even crossing count → outside, and no segment crosses →
  **kept**. The Douglas–Peucker pass keeps both loops, since they are far apart relative to 0.5.
- **L4. Precision.** Stock rasterises the lasso on the integer page grid and truncates vertex coordinates (§5.2).
  Tangleaf uses exact floating point, with a 1e-8 on-edge tolerance. _Example:_ a lasso whose edge is the
  vertical line x = 99.7, inside to the left, and a vertex at `(99.8, 50)`. Tangleaf: 99.8 > 99.7 → **outside**.
  Stock: `(int)` gives cell 99, and that cell belongs to the region if its centre, 99.5, is inside the path →
  **inside**, so the vertex is erased [inferred: pixel-centre sampling is Skia's non-antialiased fill rule, not
  read here]. The two can differ anywhere within about one unit of the boundary.
- **L5. Tiny or degenerate lassos.** Tangleaf ignores a lasso with fewer than 3 points or bounds under 2 units on
  either axis (`TL/ink-editing.ts:98-100`). Stock has no such guard. _Example:_ a 1.5 × 1.5-unit scribble over a
  dot. Tangleaf: **nothing**. Stock: the region may contain one grid cell, and if the dot's vertex truncates into
  it, the dot is **erased** [inferred].
- **L6. Simplification.** Tangleaf simplifies the lasso with Douglas–Peucker, tolerance 0.5 (`TL/ink-editing.ts:309`).
  Stock uses every SDK point, up to 20 000 [code]. The boundary moves by at most 0.5 units [inferred].
- **L7. Fragments.** Where stock cuts (L1), the fragments follow §4.3: new ids, appended at the end, one-vertex
  runs dropped. Tangleaf makes no fragments in lasso mode [code].
- **L8. Geometric shapes and fills.** Stock splits geometric shapes into `FreeShape` runs and splits fills
  (§5.3), unless "Do Not Erase Shapes" is on. Tangleaf has none [code].
- **L9. Multiple pages.** Stock applies the lasso to every visible page its bounds intersect (§5.2). Tangleaf has
  one sheet [code].

### 9.5 Render at lift

- **R1. Dirty rectangle.** Stock: union of the changed shapes' bounds + about one stroke width + 4 page units (§6).
  Tangleaf: union of the changed strokes' bounds + `1.75·w/2 + 2`, **plus** the previous overlay, which covers the
  eraser trail from the last preview grown by `r + 2` (§8). _Example:_ a long eraser swipe that touches one small
  stroke at its start. Stock repaints only around that stroke. Tangleaf also repaints the bounding box of the whole
  trail recorded in the last preview frame [code].
- **R2. Nothing erased.** Stock skips the render and the `ENABLE_POST` entirely, though the post-erase pause and
  resume still run ([03 §3.4](03-erasing.md)). Tangleaf returns the same stroke array, so it has no ink damage,
  but the overlay damage still goes out [code].

---

## 10. Open points

- Whether stock ever stores a one-point pen shape. That decides whether S4 can happen [unknown].
- The two `densityDpi` values behind the recorded widths (408 for the pixel eraser, 300 for the lasso default)
  [unknown].
- Android's fill rule and closing semantics for `Region.setPath` were taken from platform behaviour, not
  read in this pass [inferred]. A hardware check: draw a double-loop lasso around a stroke in stock and see
  whether it is erased (L3).
- What `GetReferenceShapesRequest` returns for plain pen strokes, and so whether the stroke eraser ever removes
  more than the hit strokes [unknown].
- On-screen effect of the filled-closed-shape early return in `GraphicalShapeSplitter.split`
  (`:193-197`) [unknown].
- The paint order of the page's shape list, as opposed to its list order, was not traced. P5's "above" assumes
  that list order is paint order [inferred].
