---
sidebar_position: 3
---

# 配置参考 (Config Reference)

本页面包含 rabbit-digger-pro 配置文件的完整 JSON Schema 参考。

:::info 自动生成
此页面由 JSON Schema 自动生成，最后更新时间：2025-08-19T19:40:41.836Z
:::

## 配置结构概览

rabbit-digger-pro 配置文件的主要结构：

- **id**: 配置标识符
- **net**: 网络层配置，定义各种代理和路由
- **server**: 服务器配置，定义本地监听服务
- **import**: 导入其他配置文件

## JSON Schema

```json
{
  "title": "Config",
  "type": "object",
  "properties": {
    "id": {
      "type": "string"
    },
    "import": {
      "items": {
        "type": "object",
        "anyOf": [
          {
            "title": "Clash",
            "type": "object",
            "required": [
              "source",
              "type"
            ],
            "properties": {
              "direct": {
                "type": [
                  "string",
                  "null"
                ]
              },
              "disable_proxy_group": {
                "default": false,
                "type": "boolean"
              },
              "name": {
                "title": "Nullable_String",
                "type": [
                  "string",
                  "null"
                ]
              },
              "prefix": {
                "type": [
                  "string",
                  "null"
                ]
              },
              "reject": {
                "type": [
                  "string",
                  "null"
                ]
              },
              "rule_name": {
                "type": [
                  "string",
                  "null"
                ]
              },
              "select": {
                "description": "Make all proxies in the group name",
                "default": null,
                "type": [
                  "string",
                  "null"
                ]
              },
              "source": {
                "title": "ImportSource",
                "oneOf": [
                  {
                    "type": "object",
                    "required": [
                      "path"
                    ],
                    "properties": {
                      "path": {
                        "type": "string"
                      }
                    },
                    "additionalProperties": false
                  },
                  {
                    "type": "object",
                    "required": [
                      "poll"
                    ],
                    "properties": {
                      "poll": {
                        "$ref": "#/definitions/ImportUrl"
                      }
                    },
                    "additionalProperties": false
                  },
                  {
                    "type": "object",
                    "required": [
                      "storage"
                    ],
                    "properties": {
                      "storage": {
                        "$ref": "#/definitions/ImportStorage"
                      }
                    },
                    "additionalProperties": false
                  },
                  {
                    "type": "object",
                    "required": [
                      "text"
                    ],
                    "properties": {
                      "text": {
                        "type": "string"
                      }
                    },
                    "additionalProperties": false
                  }
                ]
              },
              "type": {
                "type": "string",
                "const": "clash"
              }
            }
          },
          {
            "title": "EmptyConfig",
            "type": "object",
            "required": [
              "source",
              "type"
            ],
            "properties": {
              "name": {
                "title": "Nullable_String",
                "type": [
                  "string",
                  "null"
                ]
              },
              "source": {
                "title": "ImportSource",
                "oneOf": [
                  {
                    "type": "object",
                    "required": [
                      "path"
                    ],
                    "properties": {
                      "path": {
                        "type": "string"
                      }
                    },
                    "additionalProperties": false
                  },
                  {
                    "type": "object",
                    "required": [
                      "poll"
                    ],
                    "properties": {
                      "poll": {
                        "$ref": "#/definitions/ImportUrl"
                      }
                    },
                    "additionalProperties": false
                  },
                  {
                    "type": "object",
                    "required": [
                      "storage"
                    ],
                    "properties": {
                      "storage": {
                        "$ref": "#/definitions/ImportStorage"
                      }
                    },
                    "additionalProperties": false
                  },
                  {
                    "type": "object",
                    "required": [
                      "text"
                    ],
                    "properties": {
                      "text": {
                        "type": "string"
                      }
                    },
                    "additionalProperties": false
                  }
                ]
              },
              "type": {
                "type": "string",
                "const": "merge"
              }
            }
          },
          {
            "title": "Rhai",
            "type": "object",
            "required": [
              "source",
              "type"
            ],
            "properties": {
              "name": {
                "title": "Nullable_String",
                "type": [
                  "string",
                  "null"
                ]
              },
              "source": {
                "title": "ImportSource",
                "oneOf": [
                  {
                    "type": "object",
                    "required": [
                      "path"
                    ],
                    "properties": {
                      "path": {
                        "type": "string"
                      }
                    },
                    "additionalProperties": false
                  },
                  {
                    "type": "object",
                    "required": [
                      "poll"
                    ],
                    "properties": {
                      "poll": {
                        "$ref": "#/definitions/ImportUrl"
                      }
                    },
                    "additionalProperties": false
                  },
                  {
                    "type": "object",
                    "required": [
                      "storage"
                    ],
                    "properties": {
                      "storage": {
                        "$ref": "#/definitions/ImportStorage"
                      }
                    },
                    "additionalProperties": false
                  },
                  {
                    "type": "object",
                    "required": [
                      "text"
                    ],
                    "properties": {
                      "text": {
                        "type": "string"
                      }
                    },
                    "additionalProperties": false
                  }
                ]
              },
              "type": {
                "type": "string",
                "const": "rhai"
              }
            }
          }
        ]
      }
    },
    "net": {
      "additionalProperties": {
        "$ref": "#/definitions/Net"
      }
    },
    "server": {
      "additionalProperties": {
        "$ref": "#/definitions/Server"
      }
    }
  },
  "definitions": {
    "ImportStorage": {
      "type": "object",
      "required": [
        "folder",
        "key"
      ],
      "properties": {
        "folder": {
          "type": "string"
        },
        "key": {
          "type": "string"
        }
      }
    },
    "ImportUrl": {
      "type": "object",
      "required": [
        "url"
      ],
      "properties": {
        "interval": {
          "type": [
            "integer",
            "null"
          ],
          "format": "uint64",
          "minimum": 0
        },
        "url": {
          "type": "string"
        }
      }
    },
    "Net": {
      "type": "object",
      "anyOf": [
        {
          "title": "AliasNetConfig",
          "description": "A net refering to another net.",
          "type": "object",
          "required": [
            "net",
            "type"
          ],
          "properties": {
            "net": {
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "alias"
            }
          }
        },
        {
          "title": "EmptyConfig",
          "type": "object",
          "required": [
            "type"
          ],
          "properties": {
            "type": {
              "type": "string",
              "const": "blackhole"
            }
          }
        },
        {
          "title": "CombineNetConfig",
          "description": "CombineNet merges multiple nets into one.",
          "type": "object",
          "required": [
            "lookup_host",
            "tcp_bind",
            "tcp_connect",
            "type",
            "udp_bind"
          ],
          "properties": {
            "lookup_host": {
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "tcp_bind": {
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "tcp_connect": {
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "combine"
            },
            "udp_bind": {
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            }
          }
        },
        {
          "title": "DnsConfig",
          "type": "object",
          "required": [
            "server",
            "type"
          ],
          "properties": {
            "net": {
              "default": null,
              "anyOf": [
                {
                  "anyOf": [
                    {
                      "type": "string"
                    },
                    {
                      "$ref": "#/definitions/Net"
                    }
                  ]
                },
                {
                  "type": "null"
                }
              ]
            },
            "server": {
              "$ref": "#/definitions/net_dns_DnsServer"
            },
            "type": {
              "type": "string",
              "const": "dns"
            }
          }
        },
        {
          "title": "DNSNetConfig",
          "type": "object",
          "required": [
            "type"
          ],
          "properties": {
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "dns_sniffer"
            }
          }
        },
        {
          "title": "HttpNetConfig",
          "type": "object",
          "required": [
            "server",
            "type"
          ],
          "properties": {
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "server": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "type": {
              "type": "string",
              "const": "http"
            }
          }
        },
        {
          "title": "LocalNetConfig",
          "description": "A local network.",
          "type": "object",
          "required": [
            "type"
          ],
          "properties": {
            "bind_addr": {
              "description": "bind to address",
              "type": [
                "string",
                "null"
              ],
              "format": "ip"
            },
            "bind_device": {
              "description": "bind to device",
              "type": [
                "string",
                "null"
              ]
            },
            "connect_timeout": {
              "description": "timeout of TCP connect, in seconds.",
              "type": [
                "integer",
                "null"
              ],
              "format": "uint64",
              "minimum": 0
            },
            "lookup_host": {
              "description": "Change the default system DNS resolver to custom one.",
              "default": null,
              "anyOf": [
                {
                  "anyOf": [
                    {
                      "type": "string"
                    },
                    {
                      "$ref": "#/definitions/Net"
                    }
                  ]
                },
                {
                  "type": "null"
                }
              ]
            },
            "mark": {
              "description": "set SO_MARK on linux",
              "type": [
                "integer",
                "null"
              ],
              "format": "uint32",
              "minimum": 0
            },
            "nodelay": {
              "description": "set nodelay. default is true",
              "default": null,
              "type": [
                "boolean",
                "null"
              ]
            },
            "recv_buffer_size": {
              "description": "change the system receive buffer size of the socket. by default it remains unchanged.",
              "type": [
                "integer",
                "null"
              ],
              "format": "uint",
              "minimum": 0
            },
            "send_buffer_size": {
              "description": "change the system send buffer size of the socket. by default it remains unchanged.",
              "type": [
                "integer",
                "null"
              ],
              "format": "uint",
              "minimum": 0
            },
            "tcp_keepalive": {
              "description": "enable keepalive on TCP socket, in seconds. default is 600s. 0 means disable.",
              "default": null,
              "type": [
                "number",
                "null"
              ],
              "format": "double"
            },
            "ttl": {
              "description": "set ttl",
              "default": null,
              "type": [
                "integer",
                "null"
              ],
              "format": "uint32",
              "minimum": 0
            },
            "type": {
              "type": "string",
              "const": "local"
            }
          }
        },
        {
          "title": "EmptyConfig",
          "type": "object",
          "required": [
            "type"
          ],
          "properties": {
            "type": {
              "type": "string",
              "const": "noop"
            }
          }
        },
        {
          "title": "ObfsNetConfig",
          "type": "object",
          "oneOf": [
            {
              "type": "object",
              "required": [
                "http"
              ],
              "properties": {
                "http": {
                  "$ref": "#/definitions/net_obfs_HttpSimple"
                }
              },
              "additionalProperties": false
            },
            {
              "type": "object",
              "required": [
                "plain"
              ],
              "properties": {
                "plain": {
                  "$ref": "#/definitions/net_obfs_Plain"
                }
              },
              "additionalProperties": false
            }
          ],
          "required": [
            "type"
          ],
          "properties": {
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "obfs"
            }
          }
        },
        {
          "title": "RawNetConfig",
          "type": "object",
          "required": [
            "device",
            "ip_addr",
            "mtu",
            "type"
          ],
          "properties": {
            "device": {
              "$ref": "#/definitions/net_raw_MaybeString_for_TunTapConfig"
            },
            "ethernet_addr": {
              "type": [
                "string",
                "null"
              ]
            },
            "forward": {
              "default": false,
              "type": "boolean"
            },
            "gateway": {
              "type": [
                "string",
                "null"
              ]
            },
            "ip_addr": {
              "description": "IP Cidr",
              "type": "string"
            },
            "mtu": {
              "type": "integer",
              "format": "uint",
              "minimum": 0
            },
            "type": {
              "type": "string",
              "const": "raw"
            }
          }
        },
        {
          "title": "ResolveConfig",
          "type": "object",
          "required": [
            "net",
            "resolve_net",
            "type"
          ],
          "properties": {
            "ipv4": {
              "default": true,
              "type": "boolean"
            },
            "ipv6": {
              "default": true,
              "type": "boolean"
            },
            "net": {
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "resolve_net": {
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "resolve"
            }
          }
        },
        {
          "title": "RpcNetConfig",
          "type": "object",
          "required": [
            "server",
            "type"
          ],
          "properties": {
            "codec": {
              "default": "Cbor",
              "allOf": [
                {
                  "$ref": "#/definitions/net_rpc_Codec"
                }
              ]
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "server": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "type": {
              "type": "string",
              "const": "rpc"
            }
          }
        },
        {
          "title": "RuleNetConfig",
          "type": "object",
          "required": [
            "rule",
            "type"
          ],
          "properties": {
            "lru_cache_size": {
              "default": 32,
              "type": "integer",
              "format": "uint",
              "minimum": 0
            },
            "rule": {
              "type": "array",
              "items": {
                "$ref": "#/definitions/net_rule_RuleItem"
              }
            },
            "type": {
              "type": "string",
              "const": "rule"
            }
          }
        },
        {
          "title": "SelectNetConfig",
          "type": "object",
          "required": [
            "list",
            "selected",
            "type"
          ],
          "properties": {
            "list": {
              "type": "array",
              "items": {
                "anyOf": [
                  {
                    "type": "string"
                  },
                  {
                    "$ref": "#/definitions/Net"
                  }
                ]
              }
            },
            "selected": {
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "select"
            }
          }
        },
        {
          "title": "SSNetConfig",
          "type": "object",
          "required": [
            "cipher",
            "password",
            "server",
            "type"
          ],
          "properties": {
            "cipher": {
              "$ref": "#/definitions/net_shadowsocks_Cipher"
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "password": {
              "type": "string"
            },
            "server": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "type": {
              "type": "string",
              "const": "shadowsocks"
            },
            "udp": {
              "default": false,
              "type": "boolean"
            }
          }
        },
        {
          "title": "SNINetConfig",
          "type": "object",
          "required": [
            "type"
          ],
          "properties": {
            "force_sniff": {
              "description": "Force sniff domain. By default, only sniff connection to IP address. If set to true, will sniff all connection.",
              "default": false,
              "type": "boolean"
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "ports": {
              "description": "Ports to sniff. If not set, only 443 port will be sniffed.",
              "default": null,
              "type": [
                "array",
                "null"
              ],
              "items": {
                "type": "integer",
                "format": "uint16",
                "minimum": 0
              }
            },
            "type": {
              "type": "string",
              "const": "sni_sniffer"
            }
          }
        },
        {
          "title": "Socks5NetConfig",
          "type": "object",
          "required": [
            "server",
            "type"
          ],
          "properties": {
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "server": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "type": {
              "type": "string",
              "const": "socks5"
            }
          }
        },
        {
          "title": "TlsNetConfig",
          "type": "object",
          "required": [
            "type"
          ],
          "properties": {
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "skip_cert_verify": {
              "description": "Dangerous, but can be used to skip certificate verification.",
              "default": false,
              "type": "boolean"
            },
            "sni": {
              "description": "Override domain with SNI",
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "type": {
              "type": "string",
              "const": "tls"
            }
          }
        },
        {
          "title": "TrojanNetConfig",
          "type": "object",
          "required": [
            "password",
            "server",
            "type"
          ],
          "properties": {
            "handshake_timeout": {
              "description": "timeout of TLS handshake, in seconds.",
              "type": [
                "integer",
                "null"
              ],
              "format": "uint64",
              "minimum": 0
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "password": {
              "description": "password in plain text",
              "type": "string"
            },
            "server": {
              "description": "hostname:port",
              "type": "string"
            },
            "skip_cert_verify": {
              "description": "skip certificate verify",
              "default": false,
              "type": "boolean"
            },
            "sni": {
              "description": "sni",
              "default": null,
              "type": [
                "string",
                "null"
              ]
            },
            "type": {
              "type": "string",
              "const": "trojan"
            },
            "websocket": {
              "description": "enabled websocket support",
              "default": null,
              "anyOf": [
                {
                  "$ref": "#/definitions/net_trojan_WebSocket"
                },
                {
                  "type": "null"
                }
              ]
            }
          }
        },
        {
          "title": "TrojancNetConfig",
          "type": "object",
          "required": [
            "password",
            "server",
            "type"
          ],
          "properties": {
            "handshake_timeout": {
              "description": "timeout of TLS handshake, in seconds.",
              "type": [
                "integer",
                "null"
              ],
              "format": "uint64",
              "minimum": 0
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "password": {
              "description": "password in plain text",
              "type": "string"
            },
            "server": {
              "description": "hostname:port",
              "type": "string"
            },
            "type": {
              "type": "string",
              "const": "trojanc"
            },
            "websocket": {
              "description": "enabled websocket support",
              "default": null,
              "anyOf": [
                {
                  "$ref": "#/definitions/net_trojanc_WebSocket"
                },
                {
                  "type": "null"
                }
              ]
            }
          }
        }
      ]
    },
    "Server": {
      "type": "object",
      "anyOf": [
        {
          "title": "EchoServerConfig",
          "description": "A echo server.",
          "type": "object",
          "required": [
            "bind",
            "type"
          ],
          "properties": {
            "bind": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "listen": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "echo"
            }
          }
        },
        {
          "title": "ForwardServerConfig",
          "description": "A server that forwards all connections to target.",
          "type": "object",
          "required": [
            "bind",
            "target",
            "type"
          ],
          "properties": {
            "bind": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "listen": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "resolve_interval": {
              "description": "The interval to resolve the target address. Used in UDP mode. If not set, the target address will be resolved only once. If set to 0, the target address will be resolved every time. the unit is second.",
              "default": null,
              "type": [
                "integer",
                "null"
              ],
              "format": "uint64",
              "minimum": 0
            },
            "resolve_net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "target": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "tcp": {
              "default": null,
              "type": [
                "boolean",
                "null"
              ]
            },
            "type": {
              "type": "string",
              "const": "forward"
            },
            "udp": {
              "default": false,
              "type": "boolean"
            },
            "udp_bind": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": [
                "string",
                "null"
              ]
            }
          }
        },
        {
          "title": "HttpServerConfig",
          "type": "object",
          "required": [
            "bind",
            "type"
          ],
          "properties": {
            "auth": {
              "default": null,
              "anyOf": [
                {
                  "$ref": "#/definitions/server_http_AuthConfig"
                },
                {
                  "type": "null"
                }
              ]
            },
            "bind": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "listen": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "http"
            }
          }
        },
        {
          "title": "MixedServerConfig",
          "type": "object",
          "required": [
            "bind",
            "type"
          ],
          "properties": {
            "bind": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "listen": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "http+socks5"
            }
          }
        },
        {
          "title": "RawServerConfig",
          "type": "object",
          "required": [
            "listen",
            "type"
          ],
          "properties": {
            "listen": {
              "description": "Must be raw net.",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "raw"
            }
          }
        },
        {
          "title": "RedirServerConfig",
          "type": "object",
          "required": [
            "bind",
            "type"
          ],
          "properties": {
            "bind": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "redir"
            }
          }
        },
        {
          "title": "RpcServerConfig",
          "type": "object",
          "required": [
            "bind",
            "type"
          ],
          "properties": {
            "bind": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "codec": {
              "default": "Cbor",
              "allOf": [
                {
                  "$ref": "#/definitions/server_rpc_Codec"
                }
              ]
            },
            "listen": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "rpc"
            }
          }
        },
        {
          "title": "SSServerConfig",
          "type": "object",
          "required": [
            "bind",
            "cipher",
            "password",
            "type"
          ],
          "properties": {
            "bind": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "cipher": {
              "$ref": "#/definitions/server_shadowsocks_Cipher"
            },
            "listen": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "password": {
              "type": "string"
            },
            "type": {
              "type": "string",
              "const": "shadowsocks"
            },
            "udp": {
              "default": false,
              "type": "boolean"
            }
          }
        },
        {
          "title": "Socks5ServerConfig",
          "type": "object",
          "required": [
            "bind",
            "type"
          ],
          "properties": {
            "bind": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "listen": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "socks5"
            }
          }
        },
        {
          "title": "TProxyServerConfig",
          "type": "object",
          "required": [
            "bind",
            "type"
          ],
          "properties": {
            "bind": {
              "description": "An address contains host and port.\nFor example: example.com:80, 1.1.1.1:53, [::1]:443",
              "type": "string"
            },
            "mark": {
              "type": [
                "integer",
                "null"
              ],
              "format": "uint32",
              "minimum": 0
            },
            "net": {
              "default": "local",
              "anyOf": [
                {
                  "type": "string"
                },
                {
                  "$ref": "#/definitions/Net"
                }
              ]
            },
            "type": {
              "type": "string",
              "const": "tproxy"
            }
          }
        }
      ]
    },
    "net_dns_DnsServer": {
      "description": "A net refering to another net.",
      "oneOf": [
        {
          "type": "string",
          "enum": [
            "google",
            "cloudflare"
          ]
        },
        {
          "type": "object",
          "required": [
            "custom"
          ],
          "properties": {
            "custom": {
              "type": "object",
              "required": [
                "nameserver"
              ],
              "properties": {
                "nameserver": {
                  "type": "array",
                  "items": {
                    "type": "string"
                  }
                }
              }
            }
          },
          "additionalProperties": false
        }
      ]
    },
    "net_obfs_HttpSimple": {
      "type": "object",
      "required": [
        "host"
      ],
      "properties": {
        "host": {
          "type": "string"
        },
        "method": {
          "default": "GET",
          "type": "string"
        },
        "uri": {
          "default": "/",
          "type": "string"
        }
      }
    },
    "net_obfs_Plain": {
      "type": "null"
    },
    "net_raw_MaybeString_for_TunTapConfig": {
      "anyOf": [
        {
          "$ref": "#/definitions/net_raw_TunTapConfig"
        }
      ]
    },
    "net_raw_TunTap": {
      "type": "string",
      "enum": [
        "tap",
        "tun"
      ]
    },
    "net_raw_TunTapConfig": {
      "type": "object",
      "required": [
        "host_addr",
        "type"
      ],
      "properties": {
        "host_addr": {
          "description": "host address",
          "type": "string"
        },
        "name": {
          "type": [
            "string",
            "null"
          ]
        },
        "type": {
          "$ref": "#/definitions/net_raw_TunTap"
        }
      }
    },
    "net_rpc_Codec": {
      "type": "string",
      "enum": [
        "Json",
        "Cbor"
      ]
    },
    "net_rule_DomainMatcherMethod": {
      "type": "string",
      "enum": [
        "keyword",
        "suffix",
        "match"
      ]
    },
    "net_rule_IpCidr": {
      "type": "string"
    },
    "net_rule_RuleItem": {
      "type": "object",
      "oneOf": [
        {
          "type": "object",
          "required": [
            "domain",
            "method",
            "type"
          ],
          "properties": {
            "domain": {
              "$ref": "#/definitions/net_rule_StringList"
            },
            "method": {
              "$ref": "#/definitions/net_rule_DomainMatcherMethod"
            },
            "type": {
              "type": "string",
              "enum": [
                "domain"
              ]
            }
          }
        },
        {
          "type": "object",
          "required": [
            "ipcidr",
            "type"
          ],
          "properties": {
            "ipcidr": {
              "$ref": "#/definitions/net_rule_SingleOrVec_for_IpCidr"
            },
            "type": {
              "type": "string",
              "enum": [
                "ipcidr"
              ]
            }
          }
        },
        {
          "type": "object",
          "required": [
            "ipcidr",
            "type"
          ],
          "properties": {
            "ipcidr": {
              "$ref": "#/definitions/net_rule_SingleOrVec_for_IpCidr"
            },
            "type": {
              "type": "string",
              "enum": [
                "src_ipcidr"
              ]
            }
          }
        },
        {
          "type": "object",
          "required": [
            "country",
            "type"
          ],
          "properties": {
            "country": {
              "type": "string"
            },
            "type": {
              "type": "string",
              "enum": [
                "geoip"
              ]
            }
          }
        },
        {
          "type": "object",
          "required": [
            "type"
          ],
          "properties": {
            "type": {
              "type": "string",
              "enum": [
                "any"
              ]
            }
          }
        }
      ],
      "required": [
        "target"
      ],
      "properties": {
        "target": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "$ref": "#/definitions/Net"
            }
          ]
        }
      }
    },
    "net_rule_SingleOrVec_for_IpCidr": {
      "anyOf": [
        {
          "$ref": "#/definitions/net_rule_IpCidr"
        },
        {
          "type": "array",
          "items": {
            "$ref": "#/definitions/net_rule_IpCidr"
          }
        }
      ]
    },
    "net_rule_SingleOrVec_for_String": {
      "anyOf": [
        {
          "type": "string"
        },
        {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      ]
    },
    "net_rule_StringList": {
      "$ref": "#/definitions/net_rule_SingleOrVec_for_String"
    },
    "net_shadowsocks_Cipher": {
      "type": "string",
      "enum": [
        "none",
        "table",
        "rc4-md5",
        "aes-128-ctr",
        "aes-192-ctr",
        "aes-256-ctr",
        "aes-128-cfb1",
        "aes-128-cfb8",
        "aes-128-cfb",
        "aes-192-cfb1",
        "aes-192-cfb8",
        "aes-192-cfb",
        "aes-256-cfb1",
        "aes-256-cfb8",
        "aes-256-cfb",
        "aes-128-ofb",
        "aes-192-ofb",
        "aes-256-ofb",
        "camellia-128-ctr",
        "camellia-192-ctr",
        "camellia-256-ctr",
        "camellia-128-cfb1",
        "camellia-128-cfb8",
        "camellia-128-cfb",
        "camellia-192-cfb1",
        "camellia-192-cfb8",
        "camellia-192-cfb",
        "camellia-256-cfb1",
        "camellia-256-cfb8",
        "camellia-256-cfb",
        "camellia-128-ofb",
        "camellia-192-ofb",
        "camellia-256-ofb",
        "rc4",
        "chacha20-ietf",
        "aes-128-gcm",
        "aes-256-gcm",
        "chacha20-ietf-poly1305",
        "aes-128-ccm",
        "aes-256-ccm",
        "aes-128-gcm-siv",
        "aes-256-gcm-siv",
        "xchacha20-ietf-poly1305",
        "sm4-gcm",
        "sm4-ccm"
      ]
    },
    "net_trojan_WebSocket": {
      "type": "object",
      "required": [
        "host",
        "path"
      ],
      "properties": {
        "host": {
          "type": "string"
        },
        "path": {
          "type": "string"
        }
      }
    },
    "net_trojanc_WebSocket": {
      "type": "object",
      "required": [
        "host",
        "path"
      ],
      "properties": {
        "host": {
          "type": "string"
        },
        "path": {
          "type": "string"
        }
      }
    },
    "server_http_AuthConfig": {
      "type": "object",
      "required": [
        "password",
        "username"
      ],
      "properties": {
        "password": {
          "type": "string"
        },
        "username": {
          "type": "string"
        }
      }
    },
    "server_rpc_Codec": {
      "type": "string",
      "enum": [
        "Json",
        "Cbor"
      ]
    },
    "server_shadowsocks_Cipher": {
      "type": "string",
      "enum": [
        "none",
        "table",
        "rc4-md5",
        "aes-128-ctr",
        "aes-192-ctr",
        "aes-256-ctr",
        "aes-128-cfb1",
        "aes-128-cfb8",
        "aes-128-cfb",
        "aes-192-cfb1",
        "aes-192-cfb8",
        "aes-192-cfb",
        "aes-256-cfb1",
        "aes-256-cfb8",
        "aes-256-cfb",
        "aes-128-ofb",
        "aes-192-ofb",
        "aes-256-ofb",
        "camellia-128-ctr",
        "camellia-192-ctr",
        "camellia-256-ctr",
        "camellia-128-cfb1",
        "camellia-128-cfb8",
        "camellia-128-cfb",
        "camellia-192-cfb1",
        "camellia-192-cfb8",
        "camellia-192-cfb",
        "camellia-256-cfb1",
        "camellia-256-cfb8",
        "camellia-256-cfb",
        "camellia-128-ofb",
        "camellia-192-ofb",
        "camellia-256-ofb",
        "rc4",
        "chacha20-ietf",
        "aes-128-gcm",
        "aes-256-gcm",
        "chacha20-ietf-poly1305",
        "aes-128-ccm",
        "aes-256-ccm",
        "aes-128-gcm-siv",
        "aes-256-gcm-siv",
        "xchacha20-ietf-poly1305",
        "sm4-gcm",
        "sm4-ccm"
      ]
    }
  }
}
```

## 主要配置字段

### id
- **类型**: string
- **描述**: 配置文件的唯一标识符

### net
- **类型**: object
- **描述**: 网络层配置，定义代理节点和路由规则
- **支持的类型**: 
  - `alias`
  - `blackhole`
  - `combine`
  - `dns`
  - `dns_sniffer`
  - `http`
  - `local`
  - `noop`
  - `obfs`
  - `raw`
  - `resolve`
  - `rpc`
  - `rule`
  - `select`
  - `shadowsocks`
  - `sni_sniffer`
  - `socks5`
  - `tls`
  - `trojan`
  - `trojanc`

### server
- **类型**: object  
- **描述**: 本地服务器配置，如 HTTP/SOCKS5 代理服务
- **支持的类型**:
  - `echo`
  - `forward`
  - `http`
  - `http+socks5`
  - `raw`
  - `redir`
  - `rpc`
  - `shadowsocks`
  - `socks5`
  - `tproxy`

### import
- **类型**: array
- **描述**: 导入其他配置文件或订阅地址
- **支持的类型**:
  - `clash` - 导入 Clash 配置
  - `merge` - 合并其他配置文件

## 使用 JSON Schema

您可以在支持 JSON Schema 的编辑器中使用此 schema 来获得：

- **自动补全**: 配置字段的智能提示
- **语法检查**: 实时验证配置文件格式
- **文档提示**: 字段说明和示例

### Visual Studio Code

在 `.vscode/settings.json` 中添加：

```json
{
  "yaml.schemas": {
    "./path/to/schema.json": ["config.yaml", "*.config.yaml"]
  }
}
```

### 生成 Schema 文件

使用以下命令生成最新的 schema 文件：

```bash
rabbit-digger-pro generate-schema config-schema.json
```

## 相关文档

- [基本概念](./basic) - 了解配置的基本概念
- [配置文件格式](./format) - 查看详细的配置示例
