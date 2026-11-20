export interface S3Provider {
  id: string;
  label: string;
  /** A value the endpoint is built from (R2's account ID, Supabase's project ref). */
  param?: { label: string; placeholder: string; pattern: RegExp };
  /** `fixed` regions aren't shown (R2 is always "auto"). */
  region: { initial: string; placeholder: string; fixed?: boolean };
  endpoint: (param: string, region: string) => string | null;
  /** Where to get an access key, shown under the form. */
  keys: string;
}

export const S3_PROVIDERS: S3Provider[] = [
  {
    id: 'aws',
    label: 'Amazon S3',
    region: { initial: 'us-east-1', placeholder: 'e.g. us-east-1' },
    endpoint: () => null,
    keys: 'Create an IAM access key whose policy only allows this bucket.',
  },
  {
    id: 'r2',
    label: 'Cloudflare R2',
    param: {
      label: 'Account ID',
      placeholder: 'From the R2 overview page',
      pattern: /^https:\/\/([^./]+)\.r2\.cloudflarestorage\.com/,
    },
    region: { initial: 'auto', placeholder: 'auto', fixed: true },
    endpoint: (account) => `https://${account}.r2.cloudflarestorage.com`,
    keys: 'In R2, Manage API tokens, create an "Object Read & Write" token limited to this bucket.',
  },
  {
    id: 'supabase',
    label: 'Supabase Storage',
    param: {
      label: 'Project ref',
      placeholder: 'The ID in your project URL',
      pattern: /^https:\/\/([^./]+)\.storage\.supabase\.co\/storage\/v1\/s3/,
    },
    region: { initial: '', placeholder: 'Your project region, e.g. ap-southeast-1' },
    endpoint: (ref) => `https://${ref}.storage.supabase.co/storage/v1/s3`,
    keys: 'In Project Settings, Storage, S3 Connection: turn it on and create an access key.',
  },
  {
    id: 'b2',
    label: 'Backblaze B2',
    region: { initial: '', placeholder: "From the bucket's S3 endpoint, e.g. us-west-004" },
    endpoint: (_, region) => `https://s3.${region}.backblazeb2.com`,
    keys: 'Under Application Keys, add a key that can only access this bucket.',
  },
  {
    id: 'wasabi',
    label: 'Wasabi',
    region: { initial: 'us-east-1', placeholder: 'e.g. us-east-1' },
    endpoint: (_, region) => `https://s3.${region}.wasabisys.com`,
    keys: 'Create an access key for a sub-user whose policy only allows this bucket.',
  },
  {
    id: 'spaces',
    label: 'DigitalOcean Spaces',
    region: { initial: 'nyc3', placeholder: 'e.g. nyc3' },
    endpoint: (_, region) => `https://${region}.digitaloceanspaces.com`,
    keys: 'Under Spaces Keys, create a key limited to this Space.',
  },
  {
    id: 'custom',
    label: 'Custom (MinIO and others)…',
    region: { initial: 'us-east-1', placeholder: 'Whatever your service expects' },
    endpoint: () => null,
    keys: 'Use a key that can only reach this bucket.',
  },
];

export const providerById = (id: string | null | undefined) => S3_PROVIDERS.find((p) => p.id === id) ?? S3_PROVIDERS[0];

/** The provider for settings saved before the dropdown existed, guessed from the endpoint. */
export function guessProvider(provider: string | null | undefined, endpoint: string | null | undefined): S3Provider {
  if (provider) return providerById(provider);
  if (!endpoint) return S3_PROVIDERS[0];
  return S3_PROVIDERS.find((p) => p.param?.pattern.test(endpoint)) ?? providerById('custom');
}
