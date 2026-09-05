import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import sharp from "sharp";
import { parseImageManifest, splitImageChunks, inspectImageFile } from "../../src/supervisor/image-batch.js";
import { imageComposerMatches, imageSentTurnMatches } from "../../src/daemon/actions/images.js";

test("image composer accepts rendered paragraph spacing but rejects a restored draft or missing text", () => {
  assert.equal(imageComposerMatches("Create image\n첫 문장\n\n둘째 문장", "첫 문장\n둘째 문장"), true);
  assert.equal(imageComposerMatches("첫 문장\n\n둘째 문장\nCreate image", "첫 문장\n둘째 문장"), true);
  assert.equal(imageComposerMatches("previous draft\nCreate image\n첫 문장", "첫 문장"), false);
  assert.equal(imageComposerMatches("Create image\n첫 문장", "첫 문장\n둘째 문장"), false);
  assert.equal(imageComposerMatches("첫 문장", "첫 문장"), false);
  assert.equal(imageSentTurnMatches("context.zip Zip Archive Create image 첫 문장 둘째 문장", "첫 문장\n둘째 문장"), true);
  assert.equal(imageSentTurnMatches("Create image 첫 문장 다른 문장", "첫 문장\n둘째 문장"), false);
  assert.equal(imageSentTurnMatches("Create image 첫 문장", "첫 문장\n둘째 문장"), false);
  assert.equal(imageSentTurnMatches("anything", ""), false);
});

test("nine independent prompts become a five-image chunk and a four-image chunk", () => {
  const input = parseImageManifest({ images: [
    { id: "01", prompt: "first" }, { id: "02", prompt: "second" }, { id: "03", prompt: "third" },
    { id: "04", prompt: "fourth" }, { id: "05", prompt: "fifth" }, { id: "06", prompt: "sixth" },
    { id: "07", prompt: "seventh" }, { id: "08", prompt: "eighth" }, { id: "09", prompt: "ninth" },
  ] });
  assert.deepEqual(splitImageChunks(input.images).map((chunk) => chunk.map((item) => item.id)),
    [["01", "02", "03", "04", "05"], ["06", "07", "08", "09"]]);
});

test("invalid manifests are rejected before a browser or request can be created", () => {
  for (const input of [null, { images: [] }, { images: [{ id: "../escape", prompt: "draw" }] },
    { images: [{ id: "x", prompt: " " }] }, { images: [{ id: "x", prompt: "a" }, { id: "x", prompt: "b" }] },
    { images: [{ id: "X", prompt: "a" }, { id: "x", prompt: "b" }] },
    { images: [{ id: "x", prompt: "a", file: "/secret" }] },
  ]) assert.throws(() => parseImageManifest(input), { name: "InputError" });
});

test("download validation decodes real pixels and rejects text, broken files, and thumbnails", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-image-bytes-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const good = path.join(directory, "provider-file");
  await sharp({ create: { width: 512, height: 768, channels: 4, background: "#224466" } }).png().toFile(good);
  assert.deepEqual(await inspectImageFile(good), { extension: "png", width: 512, height: 768 });
  const bad = path.join(directory, "not-an-image.png");
  await writeFile(bad, "Here are your five images");
  await assert.rejects(() => inspectImageFile(bad));
  const tiny = path.join(directory, "thumbnail.png");
  await sharp({ create: { width: 1, height: 1, channels: 3, background: "white" } }).png().toFile(tiny);
  await assert.rejects(() => inspectImageFile(tiny), /too small/);
});
