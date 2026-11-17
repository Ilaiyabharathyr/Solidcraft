# Feature mirrors and patterns

Choose **Mirror**, **Rectangular Pattern**, **Circular Pattern** or **Pattern on Path** from the Solid
Create panel and set **Object Type** to **Features**. Select the source features
in the timeline or pick their faces in the viewport. Selecting features before
starting the command fills the Features input automatically.

Feature copies replay the source operations: a patterned hole cuts the target
solid, and an additive feature adds material. The mirror's New Body / Join
selector belongs to body copying and is hidden for feature replays.

Select the mirror plane, rectangular directions or circular axis using the
selection inputs. Click an input to pick into it; click a selected item again to
remove it, or use its clear button. The preview uses the same commands as OK.
Cancel leaves the design and undo history unchanged.

Rectangular feature patterns have two direction columns, each with Quantity,
Spacing, Flip and Center controls. Directions can meet at any angle. A missing
second direction produces a single row. Direction vectors are saved with the
feature; editing a picked guide later does not change these saved vectors.

For each direction, choose **Up to reference** to space the instances evenly
towards a selected straight edge, planar face or construction plane. For an edge,
pick one perpendicular to the pattern direction. **Offset from** is the distance
to leave before the reference, measured along that direction. The seed stays in
place. For a single hole, the offset is measured from the last hole's centre.
For bodies and other supported features it is measured from the centre of the
seed geometry's bounds. The saved reference and offset are used again on rebuild.

For example, start with a hole centred 10 mm from the near end of a 200 mm plate.
Select the far edge, set **Offset from** to 10 mm and **Quantity** to 7. The hole
centres land at 10, 40, 70, 100, 130, 160 and 190 mm.

In a rectangular, circular or path pattern, turn on **Instances to skip** to
pick individual copies in the viewport. An extrude cut with separate hole
profiles has a numbered marker at each copied opening. Each hole can be skipped
independently. Green markers stay in the pattern and red markers are skipped. Click a marker to
toggle it, or choose **Lasso** or **Box** and enclose several markers. Hold
**Shift** to skip the selected copies or **Alt** to restore them. **Skipped**
lists the copy numbers. Use its **Restore** buttons for individual copies or
**Clear skipped instances** for all copies. The original is instance 1.
A rectangular exclusion follows its grid position and source member as the
quantities change. If the position falls outside the grid, the exclusion takes
effect again when the position returns.

Circular feature patterns have Quantity, Total angle, Flip and Center controls.
Six instances over 90 degrees use 18-degree steps, including both ends of the
arc. A full circle divides 360 degrees by the quantity without duplicating the
source at 360 degrees.

Quantities include the original. Center keeps the original fixed and alternates
copies on its two sides. For four instances the slots are `0, +1, -1, +2`;
Flip changes them to `0, -1, +1, -2`. This also chooses the extra side for an even
centered circular pattern. Rectangular directions have independent controls.

Double-click a saved feature in the timeline to edit it. Mirrors and patterns
retain their feature sources, including features that created bodies, and
patterns retain their placement controls and arbitrary direction vectors after
save/reopen. Source edits recompute the copies. Undo and redo cover the whole edit.

The command forms share sketch patterns' alignment names:

```json
{"command":"solid.pattern.rectangular","params":{"features":["Hole1"],"dir1":[1,0,0],"count1":4,"spacing1":15,"dir2":[0.6,0.8,0],"count2":2,"spacing2":20,"alignment":"center","alignment2":"center","reversed":true,"reversed2":false}}
```

```json
{"command":"solid.pattern.circular","params":{"features":["Hole1"],"axis":{"origin":[40,60,0],"dir":[0,0,1]},"count":6,"angle":"90 deg","alignment":"center","reversed":true}}
```

Omitted alignments default to `right`, and omitted reversals to `false`, preserving
older designs. In spacing mode the command API also accepts `left`. With a
reference, the pattern automatically runs towards the selected geometry.
