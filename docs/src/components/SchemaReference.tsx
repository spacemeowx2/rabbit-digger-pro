import React, {useEffect, useMemo, useState} from 'react';

type JsonSchema = {
  title?: string;
  description?: string;
  type?: string | string[];
  properties?: Record<string, JsonSchema>;
  required?: string[];
  items?: JsonSchema;
  anyOf?: JsonSchema[];
  allOf?: JsonSchema[];
  oneOf?: JsonSchema[];
  const?: string;
  $ref?: string;
  definitions?: Record<string, JsonSchema>;
  additionalProperties?: JsonSchema | boolean;
};

type SchemaVariant = {
  typeName: string;
  schema?: JsonSchema;
};

type SchemaReferenceProps = {
  schemaUrl?: string;
};

const DEFAULT_SCHEMA_URL = '/schema/rabbit-digger-pro-schema.json';

function getDescription(schema?: JsonSchema): string | undefined {
  return schema?.description?.trim();
}

function normalizeSchema(schema?: JsonSchema | boolean): JsonSchema | undefined {
  if (!schema || typeof schema === 'boolean') {
    return undefined;
  }
  return schema;
}

function mergeSchema(base?: JsonSchema, override?: JsonSchema): JsonSchema | undefined {
  if (!base && !override) return undefined;
  return {
    ...base,
    ...override,
    properties: {
      ...(base?.properties ?? {}),
      ...(override?.properties ?? {}),
    },
    required: override?.required ?? base?.required,
  };
}

function collectVariants(schema?: JsonSchema | boolean, root?: JsonSchema): SchemaVariant[] {
  const normalized = normalizeSchema(schema);
  if (!normalized) return [];
  let variants = normalized.anyOf ?? normalized.oneOf ?? [];
  if (!variants.length && normalized.$ref && root?.definitions) {
    const refKey = normalized.$ref.replace('#/definitions/', '');
    const refSchema = root.definitions?.[refKey];
    if (refSchema?.anyOf || refSchema?.oneOf) {
      variants = refSchema.anyOf ?? refSchema.oneOf ?? [];
    }
  }
  return variants.map((variant) => {
    const allOf = variant.allOf ?? [];
    let typeName = variant.title;
    let merged: JsonSchema | undefined = allOf.length ? {} : variant;
    if (allOf.length) {
      allOf.forEach((item) => {
        if (!typeName && item.title) {
          typeName = item.title;
        }
        merged = mergeSchema(merged, item);
      });
    }
    if (!typeName && merged?.properties?.type?.const) {
      typeName = String(merged.properties.type.const);
    }
    return {
      typeName: typeName ?? 'unknown',
      schema: merged,
    };
  });
}

function SchemaTable({schema}: {schema?: JsonSchema}) {
  const entries = useMemo(() => {
    if (!schema?.properties) return [];
    return Object.entries(schema.properties).map(([key, value]) => ({
      key,
      description: getDescription(value),
      required: schema.required?.includes(key) ?? false,
    }));
  }, [schema]);

  if (!entries.length) {
    return <p>暂无字段信息。</p>;
  }

  return (
    <table>
      <thead>
        <tr>
          <th>字段</th>
          <th>说明</th>
          <th>必填</th>
        </tr>
      </thead>
      <tbody>
        {entries.map((entry) => (
          <tr key={entry.key}>
            <td>
              <code>{entry.key}</code>
            </td>
            <td>{entry.description ?? '—'}</td>
            <td>{entry.required ? '是' : '否'}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function SchemaSection({
  title,
  variants,
}: {
  title: string;
  variants: SchemaVariant[];
}) {
  if (!variants.length) {
    return (
      <section className="margin-bottom--lg">
        <h2>{title}</h2>
        <p>暂无可用的 Schema 定义。</p>
      </section>
    );
  }

  return (
    <section className="margin-bottom--lg">
      <h2>{title}</h2>
      {variants.map((variant) => (
        <div key={variant.typeName} className="card margin-top--md">
          <div className="card__body">
            <h3>
              <code>{variant.typeName}</code>
            </h3>
            {variant.schema?.description && (
              <p className="margin-top--sm">{variant.schema.description}</p>
            )}
            <SchemaTable schema={variant.schema} />
          </div>
        </div>
      ))}
    </section>
  );
}

export default function SchemaReference({
  schemaUrl = DEFAULT_SCHEMA_URL,
}: SchemaReferenceProps): JSX.Element {
  const [schema, setSchema] = useState<JsonSchema | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch(schemaUrl)
      .then((res) => {
        if (!res.ok) {
          throw new Error(`加载失败: ${schemaUrl} (${res.status})`);
        }
        return res.json();
      })
      .then((data) => {
        setSchema(data);
      })
      .catch((err) => {
        setError(err instanceof Error ? err.message : String(err));
      });
  }, [schemaUrl]);

  const netVariants = useMemo(
    () =>
      collectVariants(schema?.properties?.net?.additionalProperties, schema ?? undefined),
    [schema],
  );
  const serverVariants = useMemo(
    () =>
      collectVariants(schema?.properties?.server?.additionalProperties, schema ?? undefined),
    [schema],
  );
  const importVariants = useMemo(
    () => collectVariants(schema?.properties?.import?.items, schema ?? undefined),
    [schema],
  );

  return (
    <div>
      <p>
        Schema 来源：<code>{schemaUrl}</code>
      </p>
      {error && <p className="text--danger">加载 Schema 失败：{error}</p>}
      {!schema && !error && <p>正在加载 Schema...</p>}
      {schema && (
        <>
          <SchemaSection title="net 类型" variants={netVariants} />
          <SchemaSection title="server 类型" variants={serverVariants} />
          <SchemaSection title="import 类型" variants={importVariants} />
        </>
      )}
    </div>
  );
}
