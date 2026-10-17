import type { VehicleImages, VehicleStatus } from '@panda/types';

export type DoorKey = 'front_left' | 'front_right' | 'rear_left' | 'rear_right' | 'frunk' | 'rear_gate' | 'tonneau' | 'side_bin_left' | 'side_bin_right';
type Image = VehicleImages['all'][number];
type Design = 'light' | 'dark';
type Placement = 'side' | 'overhead' | 'front' | 'rear' | 'unknown';
const CLOSURES: Array<[DoorKey, Array<keyof VehicleStatus>]> = [
  ['front_left', ['door_front_left_closed']], ['front_right', ['door_front_right_closed']],
  ['rear_left', ['door_rear_left_closed']], ['rear_right', ['door_rear_right_closed']],
  ['frunk', ['closure_frunk_closed']], ['rear_gate', ['closure_liftgate_closed', 'closure_tailgate_closed']],
  ['tonneau', ['tonneau_closed']], ['side_bin_left', ['side_bin_left_closed']], ['side_bin_right', ['side_bin_right_closed']],
];
const DOOR_TOKENS: Record<DoorKey, string[][]> = {
  front_left: [['front', 'left', 'open']], front_right: [['front', 'right', 'open']],
  rear_left: [['rear', 'left', 'open']], rear_right: [['rear', 'right', 'open']], frunk: [['frunk', 'open']],
  rear_gate: [['tailgate', 'open'], ['liftgate', 'open'], ['hatch', 'open']], tonneau: [['tonneau', 'open']],
  side_bin_left: [['side', 'bin', 'left', 'open'], ['side_bin_left_open']],
  side_bin_right: [['side', 'bin', 'right', 'open'], ['side_bin_right_open']],
};
const CHARGING_TOKENS = [['charging', 'light'], ['charge', 'light'], ['charge', 'port'], ['port', 'open']];

function indexImages(images: VehicleImages['all'] = []) {
  return images.map((image) => {
    const placement = (image.placement ?? '').toLowerCase();
    const kind: Placement = placement.includes('side') ? 'side' : /overhead|top|bird/.test(placement) ? 'overhead' : placement.includes('front') ? 'front' : /rear|back/.test(placement) ? 'rear' : 'unknown';
    const text = `${image.placement ?? ''} ${image.design ?? ''} ${image.url ?? ''} ${JSON.stringify(image.metadata ?? {})}`.toLowerCase();
    const chargingPlacement = placement.includes('side') && /charging|charge/.test(placement);
    return { image, kind, text, design: (image.design ?? '').toLowerCase(), chargingPlacement, charging: chargingPlacement || /side[-_]charging/.test(text) };
  });
}
type Indexed = ReturnType<typeof indexImages>;
function firstImage(images: VehicleImages['all'] | undefined, kind: 'side' | 'overhead' | 'charging', design?: Design) {
  const candidates = indexImages(images).filter((entry) => kind === 'charging' ? entry.charging : entry.kind === kind && (kind !== 'side' || !entry.chargingPlacement));
  const preferred = design ? candidates.find((entry) => entry.design.includes(design)) : undefined;
  return preferred?.image.url || candidates[0]?.image.url;
}

/** The first match consumes a priority slot even when its URL is empty. */
function overlay(candidates: Indexed, tokenSets: string[][], design: Design) {
  const priorities: Array<Image | undefined> = Array(tokenSets.length * 2);
  for (const entry of candidates) {
    tokenSets.forEach((tokens, index) => {
      if (!tokens.every((token) => entry.text.includes(token))) return;
      priorities[index + tokenSets.length] ??= entry.image;
      if (entry.design.includes(design)) priorities[index] ??= entry.image;
    });
  }
  return priorities.find((image) => image?.url)?.url;
}
export function getOpenDoorStates(status: VehicleStatus | null | undefined): DoorKey[] {
  if (status == null) return [];
  return CLOSURES.flatMap(([door, fields]) => fields.some((field) => status[field] === false) ? [door] : []);
}
export function getDoorOverlayUrls(images: VehicleImages['all'] | undefined, openDoors: DoorKey[], designPreference: Design): string[] {
  const overhead = indexImages(images).filter((entry) => entry.kind === 'overhead');
  const urls = new Set<string>();
  for (const door of openDoors) {
    const url = overlay(overhead, DOOR_TOKENS[door] ?? [['open']], designPreference);
    if (url) urls.add(url);
  }
  return [...urls];
}
export function findFirstOverheadImage(images: VehicleImages['all'] | undefined, design?: Design): string | undefined { return firstImage(images, 'overhead', design); }
export function findFirstSideImage(images: VehicleImages['all'] | undefined, design?: Design): string | undefined { return firstImage(images, 'side', design); }
export function findSideChargingImage(images: VehicleImages['all'] | undefined, design?: Design): string | undefined { return firstImage(images, 'charging', design); }
export function findBestChargingSideOverlay(images: VehicleImages['all'] | undefined, designPreference: Design) {
  return overlay(indexImages(images).filter((entry) => entry.kind === 'side'), CHARGING_TOKENS, designPreference);
}
