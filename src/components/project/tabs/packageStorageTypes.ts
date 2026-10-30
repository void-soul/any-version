export interface PackageStorageInfo {
  path: string;
  real_target?: string | null;
  detect_source?: string | null;
  size?: string | number;
}
