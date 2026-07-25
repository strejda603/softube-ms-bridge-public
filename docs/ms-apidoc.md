# Mixing Station API (cleaned)

This is a cleaned, Markdown-first version of the Mixing Station API export.
It focuses on readable endpoint sections with fenced JSON where available.

> [!NOTE]
> This file is generated from the previous export content and intentionally avoids embedding a raw text dump.

## Index

| Method | Path | Title |
|---:|---|---|
| POST | `/app/idcas` | Creates a new IDCA |
| POST | `/app/idcas/{index}` | Modifies a IDCA |
| POST | `/app/idcas/{index}/delete` | Deletes an IDCA |
| POST | `/app/idcas/rearrange` | Updates the IDCAs order |
| GET | `/app/mixers/available` | Get mixer models |
| POST | `/app/mixers/connect` | Connect |
| GET | `/app/mixers/current` | Get currently selected mixer |
| POST | `/app/mixers/disconnect` | Disconnect |
| POST | `/app/mixers/offline` | Start offline mode |
| POST | `/app/mixers/search` | Start mixer search |
| GET | `/app/mixers/searchResults` | Get search results |
| GET | `/app/network/interfaces` | Get network interfaces |
| POST | `/app/network/interfaces/primary` | Override primary interface |
| POST | `/app/presets/channel/apply` | Recalls the given MS Preset data |
| POST | `/app/presets/channel/create` | Returns the state of a single channel as MS Preset |
| GET | `/app/presets/lastError` | Returns any error messages that occurred during the last recall |
| POST | `/app/presets/scenes/apply` | Recalls the given MS Scene data |
| POST | `/app/presets/scenes/create` | Returns the current mixer state as MS Scene |
| GET | `/app/presets/scopes` | Returns all available scopes |
| POST | `/app/save` | Saves the current app settings |
| GET | `/app/state` | Get app state |
| GET | `/app/ui/selectedChannel` | Returns the currently selected channel |
| GET | `/app/ui/selectedChannel/{nameOrIndex}` | Sets the currently selected channel, either by name or index |
| GET | `/console/auth/info` | Returns the security details about this mixer |
| POST | `/console/auth/login` | Logs in to the mixer using the given credentials. |
| GET | `/console/data/categories` | Returns all data categories. |
| GET | `/console/data/definitions/{path}` | Returns the data definitions for the given path |
| GET | `/console/data/definitions2/{path}` | Returns the data definitions for the given paths |
| GET | `/console/data/get/{path}/{format}` | Returns the current value at the given path. |
| GET | `/console/data/paths` | Returns all data paths available for the current mixer |
| GET | `/console/data/paths/{path}` | Returns a sub-path |
| POST | `/console/data/set/{path}/{format}` | Sets the value at the given path. |
| POST | `/console/data/subscribe` | Subscribe data |
| POST | `/console/data/unsubscribe` | Unsubscribe data |
| GET | `/console/information` | Returns details about the channel architecture of this mixer. |
| POST | `/console/metering/subscribe` | Subscribe metering |
| POST | `/console/metering/unsubscribe` | Unsubscribe metering |
| POST | `/console/metering2/subscribe` | Subscribe metering |
| GET | `/console/mixTargets` | Returns all signal sinks which can be used as mix target for the channels |
| GET | `/console/onConfigChanged` | Mixer config changed event |
| GET | `/convert/{path}/ntov/{val}` | Converts from normalized to unit format. |
| GET | `/convert/{path}/vton/{val}` | Converts from a unit value to a normalized value. |
| GET | `/development/crashTest` | Crash test |
| GET | `/rf/connectors` | Get connectors |
| GET | `/rf/devices` | Get all RF device config |
| POST | `/rf/devices/add` | Adds a new RF device |
| POST | `/rf/devices/remove/{uid}` | Removes a RF device |
| GET | `/rf/search/results` | Get search results |
| POST | `/rf/search/start` | Start search |
| POST | `/rf/search/stop` | Stop search |

## /app/idcas

### POST `/app/idcas`
**Creates a new IDCA**

This will add a new IDCA with the given members.Afterwards the new IDCA will appear in the data tree as 'idca.X' where X is the index of the newly created IDCA returned in the reply.

#### Request body

```json
{
	"members": [
		{
			"offset": 0,
			"type": 0
		}
	]
}
```

#### Responses

- **200** — Success

  ```json
  {
  	"members": [
  		{
  			"offset": 0,
  			"type": 0
  		}
  	],
  	"index": 0
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### POST `/app/idcas/{index}`
**Modifies a IDCA**

Modifies the members of an existing IDCA with the given index.

#### Parameters

- Name	Description
- index *
- string
- (path)

#### Request body

```json
{
	"members": [
		{
			"offset": 0,
			"type": 0
		}
	]
}
```

#### Responses

- **200** — Success

  ```json
  {
  	"members": [
  		{
  			"offset": 0,
  			"type": 0
  		}
  	],
  	"index": 0
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### POST `/app/idcas/{index}/delete`
**Deletes an IDCA**

This will delete the existing IDCA with the given index. Note that this might change the index of all IDCAs after the given index.

#### Parameters

- Name	Description
- index *
- string
- (path)

#### Responses

- **204** — No response

### POST `/app/idcas/rearrange`
**Updates the IDCAs order**

This will update the position of the IDCAs. The given list represents the source indices of the existing IDCAs

#### Request body

```json
{
	"newIndices": [
		{}
	]
}
```

#### Responses

- **200** — Success

  ```json
  {
  	"dcas": [
  		{
  			"members": [
  				{
  					"offset": 0,
  					"type": 0
  				}
  			],
  			"index": 0
  		}
  	]
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error


## /app/mixers

### GET `/app/mixers/available`
**Get mixer models**

Returns all supported mixer models.

#### Responses

- **200** — Success

  ```json
  {
  	"consoles": [
  		{
  			"consoleId": 0,
  			"models": [
  				{}
  			],
  			"supportedHardwareModels": [
  				{}
  			],
  			"manufacturerId": 0,
  			"name": "string",
  			"canSearch": true,
  			"modelEnums": [
  				{
  					"name": "string",
  					"id": 0
  				}
  			],
  			"manufacturer": "string"
  		}
  	]
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### POST `/app/mixers/connect`
**Connect**

Connects to the mixer with the given IP/Hostname and model id

#### Request body

```json
{
	"consoleId": 0,
	"ip": "string"
}
```

#### Responses

- **204** — No response

### GET `/app/mixers/current`
**Get currently selected mixer**

Returns the meta-data of the currently used mixer.

#### Responses

- **200** — Success

  ```json
  {
  	"consoleId": 0,
  	"models": [
  		{}
  	],
  	"currentModelId": 0,
  	"supportedHardwareModels": [
  		{}
  	],
  	"ipAddress": "string",
  	"manufacturerId": 0,
  	"name": "string",
  	"currentModel": "string",
  	"canSearch": true,
  	"modelEnums": [
  		{
  			"name": "string",
  			"id": 0
  		}
  	],
  	"manufacturer": "string"
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### POST `/app/mixers/disconnect`
**Disconnect**

Stops the network stack and return to the initial app state

#### Responses

- **204** — No response

### POST `/app/mixers/offline`
**Start offline mode**

Starts the offline mode for mixers of the given series and model.

#### Request body

```json
{
	"consoleId": 0,
	"modelId": 0,
	"model": "string"
}
```

#### Responses

- **204** — No response

### POST `/app/mixers/search`
**Start mixer search**

Starts searching for mixers of the given variant.

#### Request body

```json
{
	"consoleId": 0
}
```

#### Responses

- **204** — No response

### GET `/app/mixers/searchResults`
**Get search results**

Returns a list of all mixers found in the network (of the current selected series)

#### Responses

- **200** — Success

  ```json
  {
  	"results": [
  		{
  			"modelId": 0,
  			"ip": "string",
  			"name": "string",
  			"model": "string",
  			"version": "string"
  		}
  	]
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error


## /app/network

### GET `/app/network/interfaces`
**Get network interfaces**

Returns all network interfaces and their current status

#### Responses

- **200** — Success

  ```json
  {
  	"interfaces": [
  		{
  			"displayName": "string",
  			"isPrimary": true,
  			"name": "string",
  			"ipAddress": "string",
  			"subnetMask": "string",
  			"overridePrimary": true
  		}
  	]
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### POST `/app/network/interfaces/primary`
**Override primary interface**

Enforces the NIC with the given name as primary. This must be set before starting any search / connection process

#### Request body

```json
{
	"name": "string"
}
```

#### Responses

- **200** — Success

  ```json
  {
  	"interfaces": [
  		{
  			"displayName": "string",
  			"isPrimary": true,
  			"name": "string",
  			"ipAddress": "string",
  			"subnetMask": "string",
  			"overridePrimary": true
  		}
  	]
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error


## /app/presets

### POST `/app/presets/channel/apply`
**Recalls the given MS Preset data**

#### Request body

```json
{
	"data": {},
	"scope": 0,
	"channel": {
		"offset": 0,
		"type": 0
	}
}
```

#### Responses

- **204** — No response

### POST `/app/presets/channel/create`
**Returns the state of a single channel as MS Preset**

#### Request body

```json
{
	"src": {
		"offset": 0,
		"type": 0
	},
	"scope": 0
}
```

#### Responses

- **200** — Success

  ```json
  {
  	"data": {},
  	"scope": 0,
  	"channel": {
  		"offset": 0,
  		"type": 0
  	}
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### GET `/app/presets/lastError`
**Returns any error messages that occurred during the last recall**

#### Responses

- **200** — Success

  ```json
  {
  	"warnings": [
  		{}
  	],
  	"errors": [
  		{}
  	]
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### POST `/app/presets/scenes/apply`
**Recalls the given MS Scene data**

#### Request body

```json
{
	"data": {},
	"globalScope": 0,
	"channelScopes": [
		{
			"src": {
				"offset": 0,
				"type": 0
			},
			"scope": 0,
			"dest": {
				"offset": 0,
				"type": 0
			}
		}
	]
}
```

#### Responses

- **204** — No response

### POST `/app/presets/scenes/create`
**Returns the current mixer state as MS Scene**

#### Responses

- **200** — Success

  ```json
  {
  	"data": {},
  	"globalScope": 0,
  	"channelScopes": [
  		{
  			"src": {
  				"offset": 0,
  				"type": 0
  			},
  			"scope": 0,
  			"dest": {
  				"offset": 0,
  				"type": 0
  			}
  		}
  	]
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### GET `/app/presets/scopes`
**Returns all available scopes**

#### Responses

- **200** — Success

  ```json
  {
  	"channel": [
  		{
  			"name": "string",
  			"bitPos": 0
  		}
  	],
  	"global": [
  		{
  			"name": "string",
  			"bitPos": 0
  		}
  	]
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error


## /app/save

### POST `/app/save`
**Saves the current app settings**

This will persist all app settings

#### Responses

- **204** — No response


## /app/state

### GET `/app/state`
**Get app state**

Returns the current state of the app.

#### Responses

- **200** — Success

  ```json
  {
  	"msg": "string",
  	"progress": 0,
  	"state": "string",
  	"topState": "string"
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error


## /app/ui

### GET `/app/ui/selectedChannel`
**Returns the currently selected channel**

#### Responses

- **200** — Success

  ```json
  {
  	"genericName": "string",
  	"name": "string",
  	"index": 0
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### GET `/app/ui/selectedChannel/{nameOrIndex}`
**Sets the currently selected channel, either by name or index**

#### Parameters

- Name	Description
- nameOrIndex *
- string
- (path)

#### Responses

- **200** — Success

  ```json
  {
  	"genericName": "string",
  	"name": "string",
  	"index": 0
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error


## /console/auth

### GET `/console/auth/info`
**Returns the security details about this mixer**

#### Responses

- **200** — Success

  ```json
  {
  	"users": [
  		{}
  	]
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### POST `/console/auth/login`
**Logs in to the mixer using the given credentials.**

#### Request body

```json
{
	"password": "string",
	"user": "string"
}
```

#### Responses

- **200** — Success

  ```json
  {
  	"success": true
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error


## /console/data

### GET `/console/data/categories`
**Returns all data categories.**

#### Responses

- **200** — Success

  ```json
  {}
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### GET `/console/data/definitions/{path}`
**Returns the data definitions for the given path**

Warning: Deprecated

#### Parameters

- Name	Description
- path *
- string
- (path)

#### Responses

- **200** — Success

  ```json
  {
  	"definitions": {
  		"additionalProp1": {
  			"path": "string",
  			"definition": {
  				"enums": [
  					{
  						"name": "string",
  						"id": 0
  					}
  				],
  				"unit": "string",
  				"tap": true,
  				"min": 0,
  				"max": 0,
  				"delta": 0,
  				"type": "string"
  			},
  			"constraints": [
  				{}
  			]
  		},
  		"additionalProp2": {
  			"path": "string",
  			"definition": {
  				"enums": [
  					{
  						"name": "string",
  						"id": 0
  					}
  				],
  				"unit": "string",
  				"tap": true,
  				"min": 0,
  				"max": 0,
  				"delta": 0,
  				"type": "string"
  			},
  			"constraints": [
  				{}
  			]
  		},
  		"additionalProp3": {
  			"path": "string",
  			"definition": {
  				"enums": [
  					{
  						"name": "string",
  						"id": 0
  					}
  				],
  				"unit": "string",
  				"tap": true,
  				"min": 0,
  				"max": 0,
  				"delta": 0,
  				"type": "string"
  			},
  			"constraints": [
  				{}
  			]
  		}
  	}
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### GET `/console/data/definitions2/{path}`
**Returns the data definitions for the given paths**

#### Parameters

- Name	Description
- path *
- string
- (path)

#### Responses

- **200** — Success

  ```json
  {
  	"node": {
  		"defaultFilterType": 0
  	},
  	"value": {
  		"enums": [
  			{
  				"name": "string",
  				"id": 0
  			}
  		],
  		"unit": "string",
  		"tap": true,
  		"min": 0,
  		"max": 0,
  		"delta": 0,
  		"title": "string",
  		"type": "string",
  		"constraints": [
  			{}
  		]
  	}
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### GET `/console/data/get/{path}/{format}`
**Returns the current value at the given path.**

Format can be 'val' or 'norm' representing the actual value or a normalized value ranging from 0-1

#### Parameters

- Name	Description
- path *
- string
- (path)
- format *
- string
- (path)

#### Responses

- **200** — Success

  ```json
  {
  	"format": "string",
  	"value": {}
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### GET `/console/data/paths`
**Returns all data paths available for the current mixer**

#### Responses

- **200** — Success

  ```json
  {
  	"val": [
  		{}
  	],
  	"child": {
  		"additionalProp1": "string",
  		"additionalProp2": "string",
  		"additionalProp3": "string"
  	}
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### GET `/console/data/paths/{path}`
**Returns a sub-path**

#### Parameters

- Name	Description
- path *
- string
- (path)

#### Responses

- **200** — Success

  ```json
  {
  	"val": [
  		{}
  	],
  	"child": {
  		"additionalProp1": "string",
  		"additionalProp2": "string",
  		"additionalProp3": "string"
  	}
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### POST `/console/data/set/{path}/{format}`
**Sets the value at the given path.**

See 'get' for format parameter description. If the given numeric value isn't supported by the mixer it will be rounded to the closest matching value.

#### Parameters

- Name	Description
- path *
- string
- (path)
- format *
- string
- (path)

#### Request body

```json
{
	"format": "string",
	"value": {}
}
```

#### Responses

- **200** — Success

  ```json
  {
  	"format": "string",
  	"value": {}
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### POST `/console/data/subscribe`
**Subscribe data**

Subscribes to the data matching the given pattern. Can only be called from a websocket.

Creating a subscription will cause the app to send the values if a new subscription was created, as well as on every change. A single * may be used to indicate a wildcard for a single path segment.

If a value doesn't exist anymore due to a mixer configuration change (for example stereo bus gets unlinked) a value update with value=null, format=null will be sent.

#### Request body

```json
{
	"path": "string",
	"format": "string"
}
```

#### Responses

- **204** — No response

### POST `/console/data/unsubscribe`
**Unsubscribe data**

Unsubscribes the data matching the given pattern. The path must match 1:1 the path used for the subscription. Can only be called from a websocket.

#### Request body

```json
{
	"path": "string",
	"format": "string"
}
```

#### Responses

- **204** — No response


## /console/information

### GET `/console/information`
**Returns details about the channel architecture of this mixer.**

#### Responses

- **200** — Success

  ```json
  {
  	"totalChannels": 0,
  	"channelColors": [
  		{
  			"name": "string",
  			"styleClass": "string"
  		}
  	],
  	"channelTypes": [
  		{
  			"offset": 0,
  			"stereo": true,
  			"name": "string",
  			"count": 0,
  			"signalTargets": [
  				{}
  			],
  			"monoParent": "string",
  			"shortName": "string",
  			"type": 0
  		}
  	],
  	"rtaFrequencies": [
  		{}
  	],
  	"dbfsOffset": 0
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error


## /console/metering

### POST `/console/metering/subscribe`
**Subscribe metering**

Warning: Deprecated

Subscribes to the metering values of channels. The metering data will be sent to '/console/metering/{id}'. If a channel is stereo two values will be included (L/R). You can call this request multiple times, either to update an existing subscription or to subscribe to different channels with a different id.

The interval parameter defines the data rate in milliseconds (global per client).

Setting 'binary' to true will cause the reply to contain a base64 encoded string (non-padded). The format is the following:

int16 fixed float point (factor 100) big endian

#### Request body

```json
{
	"binary": true,
	"interval": 0,
	"id": 0,
	"channelIndices": [
		{}
	]
}
```

#### Responses

- **204** — No response

### POST `/console/metering/unsubscribe`
**Unsubscribe metering**

Unsubscribes the metering request with the given id

#### Request body

```json
{
	"id": 0
}
```

#### Responses

- **204** — No response


## /console/metering2

### POST `/console/metering2/subscribe`
**Subscribe metering**

Subscribes to the metering values. The detailed description about this endpoint is in the manual

#### Request body

```json
{
	"binary": true,
	"interval": 0,
	"id": 0,
	"params": [
		{
			"index": 0,
			"type": 0
		}
	]
}
```

#### Responses

- **204** — No response


## /console/mixTargets

### GET `/console/mixTargets`
**Returns all signal sinks which can be used as mix target for the channels**

#### Responses

- **200** — Success

  ```json
  {
  	"targets": [
  		{
  			"isChannel": true,
  			"name": "string",
  			"channelType": {
  				"offset": 0,
  				"stereo": true,
  				"name": "string",
  				"count": 0,
  				"signalTargets": [
  					{}
  				],
  				"monoParent": {
  					"offset": 0,
  					"stereo": true,
  					"name": "string",
  					"count": 0,
  					"signalTargets": [
  						{}
  					],
  					"monoParent": "string",
  					"shortName": "string",
  					"type": 0
  				},
  				"shortName": "string",
  				"type": 0
  			},
  			"id": 0,
  			"channelIndex": 0
  		}
  	]
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error


## /console/onConfigChanged

### GET `/console/onConfigChanged`
**Mixer config changed event**

Websocket only. Gets broadcast to all clients if the mixer configuration has been changed. This may happen for mixers which have configurable mono/stereo channel counts.

#### Responses

- **204** — No response


## /convert/{path}

### GET `/convert/{path}/ntov/{val}`
**Converts from normalized to unit format.**

Converts the given normalized value using the convertion used at the given path to the actual value

#### Parameters

- Name	Description
- path *
- string
- (path)
- val *
- string
- (path)

#### Responses

- **200** — Success

  ```json
  {
  	"format": "string",
  	"value": {}
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### GET `/convert/{path}/vton/{val}`
**Converts from a unit value to a normalized value.**

Converts the given unit value using the convertion used at the given path to a normalized value

#### Parameters

- Name	Description
- path *
- string
- (path)
- val *
- string
- (path)

#### Responses

- **200** — Success

  ```json
  {
  	"format": "string",
  	"value": {}
  }
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error


## /development/crashTest

### GET `/development/crashTest`
**Crash test**

Creates an unhandled exception to test the crash behavior

#### Responses

- **204** — No response


## /rf/connectors

### GET `/rf/connectors`
**Get connectors**

Returns all RF connectors that are available

#### Responses

- **200** — Success

  ```json
  {}
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error


## /rf/devices

### GET `/rf/devices`
**Get all RF device config**

#### Responses

- **200** — Success

  ```json
  {}
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### POST `/rf/devices/add`
**Adds a new RF device**

When adding via search use the searchId!

#### Request body

```json
{
	"hostname": "string",
	"searchId": "string",
	"connectorId": 0
}
```

#### Responses

- **204** — No response

### POST `/rf/devices/remove/{uid}`
**Removes a RF device**

#### Parameters

- Name	Description
- uid *
- string
- (path)

#### Responses

- **204** — No response


## /rf/search

### GET `/rf/search/results`
**Get search results**

Returns a list of all rf devices found in the network (of the currently selected connector)

#### Responses

- **200** — Success

  ```json
  {}
  ```
- **400** — API error

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **404** — Item not found

  ```json
  {
  	"errorMsg": "string"
  }
  ```
- **500** — Unhandled error

### POST `/rf/search/start`
**Start search**

Starts searching for rf devices using the given connector id

#### Request body

```json
{
	"connectorId": 0
}
```

#### Responses

- **204** — No response

### POST `/rf/search/stop`
**Stop search**

Stops searching for devices

#### Responses

- **204** — No response

