// Images picked, pasted or dropped by the user (composer, card editor), read as base64.

import type { ImageInput } from './protocol';

export interface PickedImage {
  /** File name; pasted screenshots have a generic one. */
  name: string;
  image: ImageInput;
}

/** Reads an image file; `undefined` for anything that is not an image. */
export function readImage(file: File): Promise<PickedImage | undefined> {
  if (!file.type.startsWith('image/')) return Promise.resolve(undefined);
  return new Promise((resolve) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result);
      resolve({ name: file.name || 'pasted image', image: { mimeType: file.type, data: url.slice(url.indexOf(',') + 1) } });
    };
    reader.onerror = () => resolve(undefined);
    reader.readAsDataURL(file);
  });
}

/** The image files of a paste, a drop or a file input. */
export function imageFiles(list: FileList | null | undefined): File[] {
  return [...(list ?? [])].filter((f) => f.type.startsWith('image/'));
}

/** Calls `add` for each image the user pastes; the paste is consumed only when it has images. */
export function onImagePaste(e: ClipboardEvent, add: (image: PickedImage) => void) {
  const files = imageFiles(e.clipboardData?.files);
  if (!files.length) return;
  e.preventDefault();
  for (const f of files) void readImage(f).then((im) => im && add(im));
}

/** Size of base64 data in bytes. */
export const base64Bytes = (data: string) => Math.floor((data.length * 3) / 4) - (data.endsWith('==') ? 2 : data.endsWith('=') ? 1 : 0);
