/** Options for opening a VCF/BCF reader. */
export type ReaderOptions = {};

/** Options for creating a VCF/BCF writer. */
export type WriterOptions = {
  /** Output format: "vcf" or "bcf". Inferred from file extension if not specified. */
  format?: "vcf" | "bcf";
  /** If true, write uncompressed output. */
  uncompressed?: boolean;
  /** Number of compression threads. */
  threads?: number;
};

/**
 * Parsed genotype information for a single sample.
 *
 * Examples:
 * - `0/1` -> `{ alleles: [0, 1], phase: [false] }`
 * - `1|1` -> `{ alleles: [1, 1], phase: [true] }`
 * - `./1` -> `{ alleles: [null, 1], phase: [false] }`
 * - `1` (haploid) -> `{ alleles: [1], phase: [] }`
 * - `0/1|2` -> `{ alleles: [0, 1, 2], phase: [false, true] }`
 */
export type Genotype = {
  /** Allele indices. `null` represents a missing allele (`.`). */
  alleles: Array<number | null>;
  /**
   * Phase separators. `phase[i]` indicates whether `alleles[i+1]` is phased
   * with `alleles[i]` (`true` = `|`, `false` = `/`).
   * Length is always `alleles.length - 1` (or 0 for haploid).
   */
  phase: boolean[];
};

/** VCF/BCF header containing metadata and field definitions. */
export class Header {
  /** Get all header records (INFO, FORMAT, FILTER, contig, etc.). */
  records(): HeaderRecord[];
  /** Get a specific INFO or FORMAT field definition by ID. */
  get(section: "INFO" | "FORMAT", id: string): HeaderGetResult | undefined;
  /** Add a new INFO field definition to the header. */
  addInfo(id: string, number: string, type: "Flag" | "Integer" | "Float" | "String", description: string): void;
  /** Add a new FORMAT field definition to the header. */
  addFormat(id: string, number: string, type: "Flag" | "Integer" | "Float" | "String", description: string): void;
  /** Get the list of sample names. */
  samples(): string[];
  /** Convert header to VCF header text. */
  toString(): string;
}

/** Result from Header.get() containing field metadata. */
export type HeaderGetResult = {
  id: string;
  type: "Flag" | "Integer" | "Float" | "String";
  number: string;
  description: string;
};

/** A header record representing an INFO, FORMAT, or FILTER field definition. */
export type HeaderRecord = {
  /** Record category: "INFO", "FORMAT", or "FILTER". */
  section: "INFO" | "FORMAT" | "FILTER";
  /** Field ID (e.g., "DP", "GT"). */
  id: string;
  /** Number specification ("1", "A", "R", "G", "."). */
  number: string;
  /** Value type ("Integer", "Float", "String", "Flag"). */
  type: "Integer" | "Float" | "String" | "Flag";
  /** Field description from header. */
  description: string;
};

/** A single VCF/BCF variant record. */
export class Variant {
  /** Chromosome name (e.g., "chr1"). */
  get chrom(): string;
  /** Reference sequence ID (integer index), or null if not set. */
  get rid(): number | null;
  /** 1-based position. */
  get pos(): number;
  /** 0-based start position. */
  get start(): number;
  /** 0-based end position (exclusive). */
  get stop(): number;

  /** Variant ID (e.g., "rs12345"), or "." if not set. */
  get id(): string;
  set id(v: string);
  /** Reference allele. */
  get ref(): string;
  /** Alternate alleles. */
  get alt(): string[];
  /** Quality score, or null if missing. */
  get qual(): number | null;
  set qual(v: number | null);
  /** Filter status (e.g., ["PASS"] or ["q10", "dp"]). */
  get filter(): string[];
  set filter(v: string[]);

  /** Get an INFO field value by tag name. Returns undefined if absent. */
  info(tag: string): boolean | number | string | Array<number | string> | null | undefined;
  /** Set an INFO field value. Pass null/undefined to clear. */
  set_info(tag: string, value: boolean | number | string | Array<boolean | number | string> | null | undefined): void;
  /** Re-associate this variant with a new header (required after adding fields). */
  translate(header: Header): void;
  /** Get a FORMAT field value (array with one entry per sample). */
  format(tag: string): Array<boolean | number | string | Array<number | string> | null> | undefined;
  /** Get all FORMAT fields for a single sample by name. Includes parsed `genotype` if GT is present. */
  sample(name: string): ({ sample_name: string; genotype?: Genotype } & Record<string, number | string | null | Array<number | string | null>>) | undefined;
  /** Get all FORMAT fields for all samples, or a subset if specified. Includes parsed `genotype` if GT is present. */
  samples(subset?: string[]): Array<{ sample_name: string; genotype?: Genotype } & Record<string, number | string | null | Array<number | string | null>>>;
  /** Get parsed genotypes for all samples, or a subset if specified. */
  genotypes(subset?: string[]): Genotype[];
  /** Convert to VCF line (without trailing newline). */
  toString(): string;
}

/** Writer for creating VCF/BCF files. */
export class Writer {
  /**
   * Create a new writer.
   * @param path Output path, or "-" for stdout.
   * @param header Header to use for the output file.
   * @param opts Writer options.
   */
  constructor(path: string, header: Header, opts?: WriterOptions);
  /**
   * The header used by this writer.
   * Note: This is a snapshot of the header at construction time.
   * Later modifications to the original header will not be reflected here.
   */
  get header(): Header;
  /** Write a variant record. The variant is consumed and cannot be reused. */
  write(variant: Variant): void;
  /** Close the writer and flush any buffered data. */
  close(): void;
}

/** Reader for VCF/BCF files. */
export class Reader {
  /**
   * Open a VCF/BCF file synchronously.
   * @param path Path to the VCF/BCF file.
   * @param opts Reader options.
   */
  constructor(path: string, opts?: ReaderOptions);

  /**
   * The header for this VCF/BCF file.
   * Note: Modifications to this header (e.g., addInfo) should be done before
   * passing it to a Writer, as the Writer receives a snapshot at construction time.
   */
  get header(): Header;

  /** Returns true if an index file (.tbi or .csi) is available. */
  hasIndex(): boolean;

  /** Query a genomic region (requires index). Uses 1-based region string. */
  query(region: string): Promise<void>;
  /** Query a genomic region (requires index). Uses 0-based coordinates. */
  query(chrom: string, start0: number, end0?: number): Promise<void>;

  /** Async iterator protocol. */
  [Symbol.asyncIterator](): AsyncIterator<Variant>;
  /** Sync iterator protocol. */
  [Symbol.iterator](): Iterator<Variant>;
  /** Read next variant asynchronously. */
  next(): Promise<IteratorResult<Variant>>;
  /** Read next variant synchronously. */
  nextSync(): IteratorResult<Variant>;

  /** Close the reader and release resources. */
  close(): void;
}

/**
 * Open a VCF/BCF file asynchronously.
 * @param path Path to the VCF/BCF file.
 * @param opts Reader options.
 */
export function openReader(path: string, opts?: ReaderOptions): Promise<Reader>;
