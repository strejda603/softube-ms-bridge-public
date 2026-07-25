Metering

Using the websocket API you can also request metering data from mixing station.
Subscribe

To receive metering data, you'll need to subscribe to the metering data:

{
  "path": "/console/metering2/subscribe",
  "method": "POST",
  "body": {
    "id": 0,
    "interval": 100,
    "binary": true,
    "params": [
      {
        "type": 0,
        "index": 0
      }
    ]
  }
}

The request payload is defined as:
Field 	Description
id 	ID which identifies the subscription
interval 	The interval parameter defines the data rate in milliseconds (global per client, last one wins, min:30, max:1000)
binary 	true will cause the reply to contain a base64 encoded string (non-padded).
params.*.type 	Metering type
params.*.index 	Index of the type, out of bound requests will be ignored

A single subscription may return a maximum of 500 metering values.

To unsubscribe / clear a subscription you can either call subscribe again with an empty params list, or call

POST /console/metering/unsubscribe {id: 0}

Metering type

The metering type defines what type of metering will be sent back. This also defines the data format for this requested meter.
Type 	Description
0 	Channel mixer meters. This will be the input/output levels, as shown by mixing station in the mixer.
1 	Same as 0 but with 4 additional meters for Gate/Dyn SC Input and GR
10 	RTA
11 	RF channel meters. The index is defined as: rfDeviceId<<8\|rfChannelId

The data will be sent via websocket to the path /console/metering2/{id}
Json format

The binary=false response payload is defined as:

{
  "path": "/console/metering2/0",
  "body": {
    "v": [
      [
        -20
      ]
    ]
  }
}

where v is an array containing the responses of all params.* in the subscription. Each entry in this array is another array containing all values of that single parameter. So for a stereo channel it will include 2 values, for a mono channel 1.

The order of the values in the array is as follows:

Type 0: ch-meter-L, [ch-meter-R]
Type 1: ch-meter-L, [ch-meter-R] [gate input, gate gr, comp input, comp gr]
        If a channel has no gate, nor dynamics none of the parameters will be included.
        If it only has a gate, only the gate part will be included.
        If it only has a comp, both the gate and comp part will be included (so the values can be differentiated).
Type 10: rta-band-0, rta-band-1, ...
Type 11: rfch-audio-level(in dbfs), rfch-rf-level(in dBm)

Where [...] denotes an optional value which will only be included if available.
All values are in dB, gain reduction values are negative.

Inactive channels (for example if a mixer merges two channels into one) will still be included in the request.

Binary format

The binary format follows the same value order as defined in the Json format above, however instead of using arrays, all values will be encoded using non-padded base64, where each value is encoded as:

int16 big endian, scaled by 100 (so 1.02dB -> 102)

The binary=true response payload is defined as:

{
  "path": "/console/metering2/0",
  "body": {
    "b": "...base64..."
  }
}

RTA

The number of RTA bands and their frequencies can be queried using the console information endpoint.