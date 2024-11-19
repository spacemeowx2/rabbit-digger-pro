import React from 'react';
import clsx from 'clsx';
import styles from './styles.module.css';

// 导入 SVG 图标
import ReloadIcon from '@site/static/img/features/reload.svg';
import ConfigIcon from '@site/static/img/features/config.svg';
import SchemaIcon from '@site/static/img/features/schema.svg';
import ProtocolIcon from '@site/static/img/features/protocol.svg';
import RuleIcon from '@site/static/img/features/rule.svg';
import ApiIcon from '@site/static/img/features/api.svg';
import ClashIcon from '@site/static/img/features/clash.svg';
import PluginIcon from '@site/static/img/features/plugin.svg';
import CrossPlatformIcon from '@site/static/img/features/cross-platform.svg';

type FeatureItem = {
  title: string;
  description: JSX.Element;
  icon: React.ComponentType<React.ComponentProps<'svg'>>;
};

const FeatureList: FeatureItem[] = [
  {
    title: '热重加载',
    description: (
      <>实时生效的配置更新，无需重启程序即可应用更改</>
    ),
    icon: ReloadIcon,
  },
  {
    title: '灵活配置',
    description: (
      <>代理可以随意嵌套组合，完整支持 TCP 和 UDP 转发</>
    ),
    icon: ConfigIcon,
  },
  {
    title: 'JSON Schema 生成',
    description: (
      <>无需查文档，通过代码补全直接编写配置文件</>
    ),
    icon: SchemaIcon,
  },
  {
    title: '多协议支持',
    description: (
      <>支持 Shadowsocks、Trojan、HTTP、SOCKS5 等多种代理协议</>
    ),
    icon: ProtocolIcon,
  },
  {
    title: '规则路由',
    description: (
      <>强大的分流规则系统，支持域名、IP、GeoIP 等多种匹配方式</>
    ),
    icon: RuleIcon,
  },
  {
    title: 'API 控制',
    description: (
      <>提供 HTTP API 接口，支持程序化控制和状态监控</>
    ),
    icon: ApiIcon,
  },
  {
    title: 'Clash 订阅',
    description: (
      <>兼容 Clash 配置格式，可直接导入现有的 Clash 配置</>
    ),
    icon: ClashIcon,
  },
  {
    title: '插件系统',
    description: (
      <>可扩展的插件架构，支持自定义协议和功能</>
    ),
    icon: PluginIcon,
  },
  {
    title: '跨平台支持',
    description: (
      <>支持 Windows、Linux、macOS 等主流操作系统</>
    ),
    icon: CrossPlatformIcon,
  },
];

function Feature({ title, description, icon: Icon }: FeatureItem) {
  return (
    <div className={clsx('col col--4 margin-vert--md')}>
      <div className="text--center padding-horiz--md">
        <Icon className={styles.featureSvg} />
        <h3>{title}</h3>
        <p>{description}</p>
      </div>
    </div>
  );
}

export default function HomepageFeatures(): JSX.Element {
  return (
    <section className={styles.features}>
      <div className="container">
        <div className="row">
          {FeatureList.map((props, idx) => (
            <Feature key={idx} {...props} />
          ))}
        </div>
      </div>
    </section>
  );
}
