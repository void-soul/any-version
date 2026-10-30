import type { DbField } from "./types";

export type DbTableNavigationRow = {
      id: string;
      name: string;
      color: string;
      depth: number;
      related: boolean;
      relCount: number;
      comment: string;
      fields: DbField[];
      indexCount: number;
    };
