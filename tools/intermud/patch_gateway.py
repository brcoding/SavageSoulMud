"""Pinned gateway compatibility: request complete discovery lists on fresh startup."""
from pathlib import Path
import sys
root=Path(sys.argv[1]);path=root/'src/gateway.py';text=path.read_text()
for key in ('old_mudlist_id','old_chanlist_id'):
 text=text.replace(key+' = 0 ',key+' = -1 ')
path.write_text(text)
print('Gateway full-list startup compatibility applied.')

# Expose incoming discovery requests to the authenticated game adapter. These
# notices never go back onto I3 and carry no response data or account details.
path = root / 'src/api/events.py'
text = path.read_text()
if 'REQUEST_RECEIVED = ' not in text:
 text = text.replace('    # System Events', '    REQUEST_RECEIVED = "request_received"\n\n    # System Events', 1)
path.write_text(text)
path = root / 'src/api/event_bridge.py'
text = path.read_text()
if 'async def _process_request_notice(' not in text:
 text = text.replace('            # Route based on packet type', '''            if packet.packet_type in (PacketType.WHO_REQ, PacketType.FINGER_REQ):
                await self._process_request_notice(packet)

            # Route based on packet type''', 1)
 text += '''
'''
 # Insert inside the class, before its existing tell handler.
 text = text.replace('    async def _process_tell(', '''    async def _process_request_notice(self, packet):
        event = event_dispatcher.create_event(
            EventType.REQUEST_RECEIVED,
            {"from_mud": packet.originator_mud,
             "from_user": packet.originator_user or "",
             "to_mud": packet.target_mud,
             "request": packet.packet_type.value},
            priority=7, ttl=15,
        )
        await event_dispatcher.dispatch(event)
        self.stats["events_generated"] += 1

    async def _process_tell(''', 1)
path.write_text(text)
print('Gateway incoming-request audit events applied.')
