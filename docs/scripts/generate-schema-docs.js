#!/usr/bin/env node

const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');

/**
 * Generate JSON schema documentation for rabbit-digger-pro configuration
 */
async function generateSchemaDocs() {
  try {
    console.log('Generating JSON schema documentation...');

    // Path to the binary (relative to docs directory) - try release first, then debug
    const releaseBinaryPath = path.resolve(__dirname, '../../target/release/rabbit-digger-pro');
    const debugBinaryPath = path.resolve(__dirname, '../../target/debug/rabbit-digger-pro');
    
    let binaryPath;
    if (fs.existsSync(releaseBinaryPath)) {
      binaryPath = releaseBinaryPath;
      console.log('Using release binary...');
    } else if (fs.existsSync(debugBinaryPath)) {
      binaryPath = debugBinaryPath;
      console.log('Using debug binary...');
    } else {
      throw new Error(`Binary not found. Please run 'cargo build' or 'cargo build --release' first.`);
    }

    // Generate schema JSON
    console.log('Running schema generation...');
    const schemaJson = execSync(`"${binaryPath}" generate-schema`, { 
      encoding: 'utf8',
      cwd: path.resolve(__dirname, '../..')
    });

    // Parse the schema to format it nicely
    const schema = JSON.parse(schemaJson);

    // Generate markdown content
    const markdownContent = generateMarkdownFromSchema(schema);

    // Write to config reference file
    const outputPath = path.resolve(__dirname, '../docs/config/reference.md');
    fs.writeFileSync(outputPath, markdownContent, 'utf8');

    console.log(`✅ Schema documentation generated at ${outputPath}`);

  } catch (error) {
    console.error('❌ Error generating schema documentation:', error.message);
    process.exit(1);
  }
}

/**
 * Convert JSON schema to markdown documentation
 */
function generateMarkdownFromSchema(schema) {
  const timestamp = new Date().toISOString();
  
  let markdown = `---
sidebar_position: 3
---

# 配置参考 (Config Reference)

本页面包含 rabbit-digger-pro 配置文件的完整 JSON Schema 参考。

:::info 自动生成
此页面由 JSON Schema 自动生成，最后更新时间：${timestamp}
:::

## 配置结构概览

rabbit-digger-pro 配置文件的主要结构：

- **id**: 配置标识符
- **net**: 网络层配置，定义各种代理和路由
- **server**: 服务器配置，定义本地监听服务
- **import**: 导入其他配置文件

## JSON Schema

\`\`\`json
${JSON.stringify(schema, null, 2)}
\`\`\`

## 主要配置字段

### id
- **类型**: string
- **描述**: 配置文件的唯一标识符

### net
- **类型**: object
- **描述**: 网络层配置，定义代理节点和路由规则
- **支持的类型**: 
`;

  // Extract net types from schema definitions
  if (schema.definitions && schema.definitions.Net) {
    const netDef = schema.definitions.Net;
    if (netDef.anyOf) {
      netDef.anyOf.forEach(netType => {
        if (netType.properties && netType.properties.type && netType.properties.type.const) {
          markdown += `  - \`${netType.properties.type.const}\`\n`;
        }
      });
    }
  }

  markdown += `
### server
- **类型**: object  
- **描述**: 本地服务器配置，如 HTTP/SOCKS5 代理服务
- **支持的类型**:
`;

  // Extract server types from schema definitions
  if (schema.definitions && schema.definitions.Server) {
    const serverDef = schema.definitions.Server;
    if (serverDef.anyOf) {
      serverDef.anyOf.forEach(serverType => {
        if (serverType.properties && serverType.properties.type && serverType.properties.type.const) {
          markdown += `  - \`${serverType.properties.type.const}\`\n`;
        }
      });
    }
  }

  markdown += `
### import
- **类型**: array
- **描述**: 导入其他配置文件或订阅地址
- **支持的类型**:
`;

  // Add import types if available
  markdown += `  - \`clash\` - 导入 Clash 配置
  - \`merge\` - 合并其他配置文件

## 使用 JSON Schema

您可以在支持 JSON Schema 的编辑器中使用此 schema 来获得：

- **自动补全**: 配置字段的智能提示
- **语法检查**: 实时验证配置文件格式
- **文档提示**: 字段说明和示例

### Visual Studio Code

在 \`.vscode/settings.json\` 中添加：

\`\`\`json
{
  "yaml.schemas": {
    "./path/to/schema.json": ["config.yaml", "*.config.yaml"]
  }
}
\`\`\`

### 生成 Schema 文件

使用以下命令生成最新的 schema 文件：

\`\`\`bash
rabbit-digger-pro generate-schema config-schema.json
\`\`\`

## 相关文档

- [基本概念](./basic) - 了解配置的基本概念
- [配置文件格式](./format) - 查看详细的配置示例
`;

  return markdown;
}

// Run the script
if (require.main === module) {
  generateSchemaDocs();
}

module.exports = { generateSchemaDocs };