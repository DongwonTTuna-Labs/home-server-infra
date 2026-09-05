import sharp from "sharp";
import { InputError } from "../shared/errors.js";
import type { ImageManifest, ImagePrompt } from "../shared/types.js";

export const IMAGE_CHUNK_SIZE = 5;
export function parseImageManifest(value: unknown): ImageManifest {
  if (!value || typeof value !== "object" || Array.isArray(value)
    || Object.keys(value).some((key) => key !== "images") || !("images" in value)
    || !Array.isArray(value.images) || value.images.length === 0) {
    throw new InputError("image manifest must contain a nonempty images array");
  }
  const ids = new Set<string>();
  const images: ImagePrompt[] = [];
  for (const item of value.images) {
    if (!item || typeof item !== "object" || Array.isArray(item)
      || Object.keys(item).some((key) => key !== "id" && key !== "prompt")
      || typeof item.id !== "string" || !/^[a-z0-9][a-z0-9_-]{0,79}$/iu.test(item.id)
      || typeof item.prompt !== "string" || !item.prompt.trim() || ids.has(item.id.toLowerCase())) {
      throw new InputError("each image needs a unique safe id and a nonempty prompt; only id and prompt are allowed");
    }
    ids.add(item.id.toLowerCase());
    images.push({ id: item.id, prompt: item.prompt });
  }
  return { images };
}
export function splitImageChunks(images: readonly ImagePrompt[]): ImagePrompt[][] {
  const chunks: ImagePrompt[][] = [];
  for (let index = 0; index < images.length; index += IMAGE_CHUNK_SIZE) chunks.push(images.slice(index, index + IMAGE_CHUNK_SIZE));
  return chunks;
}
export function imageChunkPrompt(batchId: string, index: number, items: readonly ImagePrompt[]): string {
  return `Image batch ${batchId}, chunk ${index + 1}.\n`
    + `Generate exactly ${items.length} SEPARATE original images using ChatGPT's image generation tool. `
    + "Call the image tool for each requested image. Do not create a collage, contact sheet, HTML/SVG screenshot, or Python-drawn substitute. "
    + "Finish every image in this chunk in this response, preserving the order below. "
    + "Label the outputs with the specified IDs outside the images. Deliver actual downloadable image cards, not links you invent or a written claim of completion. "
    + "The attachments contain the full production context and prompt manifest; use only the IDs listed in this chunk. "
    + "If required context is missing or the image tool cannot complete the images, say so plainly.\n\n"
    + items.map((item, i) => `IMAGE ${i + 1} — ID ${item.id}\n${item.prompt}`).join("\n\n");
}
export async function inspectImageFile(filename: string): Promise<{ extension: string; width: number; height: number }> {
  const image = sharp(filename, { limitInputPixels: 64 * 1024 * 1024, failOn: "warning" });
  const meta = await image.metadata();
  if (!meta.format || !["png", "jpeg", "webp"].includes(meta.format) || (meta.pages ?? 1) !== 1) {
    throw new Error("download is not a single PNG, JPEG, or WebP image");
  }
  if (!meta.width || !meta.height || meta.width < 256 || meta.height < 256) throw new Error("downloaded image is too small");
  await image.stats(); // 헤더와 확장자가 아니라 전체 픽셀이 실제로 디코딩되는지 확인한다.
  return { extension: meta.format === "jpeg" ? "jpg" : meta.format, width: meta.width, height: meta.height };
}
