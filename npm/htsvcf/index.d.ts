export type ReaderOptions = {};

export class Header {
  records(): HeaderRecord[];
  get(section: "INFO" | "FORMAT", id: string): HeaderGetResult | undefined;
  addInfo(id: string, number: string, type: "Flag" | "Integer" | "Float" | "String", description: string): void;
  addFormat(id: string, number: string, type: "Flag" | "Integer" | "Float" | "String", description: string): void;
  toString(): string;
}

export type HeaderGetResult = {
  id: string;
  type: "Flag" | "Integer" | "Float" | "String";
  number: string;
  description: string;
};

export type HeaderRecord =
  | { type: "INFO"; key: string; [k: string]: string }
  | { type: "FORMAT"; key: string; [k: string]: string }
  | { type: "FILTER"; key: string; [k: string]: string }
  | { type: "contig"; key: string; [k: string]: string }
  | { type: "structured"; key: string; [k: string]: string }
  | { type: "generic"; key: string; value: string };

export class Variant {
  get chrom(): string;
  get rid(): number | null;
  get pos(): number;
  get start(): number;
  get stop(): number;

  get id(): string;
  set id(v: string);
  get ref(): string;
  get alt(): string[];
  get qual(): number | null;
  set qual(v: number | null);
  get filter(): string[];
  set filter(v: string[]);

  info(tag: string): boolean | number | string | Array<number | string> | null | undefined;
  set_info(tag: string, value: boolean | number | string | Array<boolean | number | string> | null | undefined): void;
  format(tag: string): Array<boolean | number | string | Array<number | string> | null> | undefined;
  toString(): string;
}

export class Reader {
  constructor(path: string, opts?: ReaderOptions);

  get header(): Header;

  hasIndex(): boolean;

  query(region: string): Promise<void>;
  query(chrom: string, start0: number, end0?: number): Promise<void>;

  [Symbol.asyncIterator](): AsyncIterator<Variant>;
  next(): Promise<IteratorResult<Variant>>;
  nextSync(): IteratorResult<Variant>;

  close(): void;
}

export function openReader(path: string, opts?: ReaderOptions): Promise<Reader>;
