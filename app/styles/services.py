import aioboto3
from fastapi import Depends, APIRouter, Query, Response, HTTPException
from uuid import UUID
from app.crud import CRUD
from app.db import get_session, AsyncSession
from app.styles.models import Style, StyleRead, StyleCreate, StyleUpdate
from app.s3.services import get_s3


router = APIRouter()


crud = CRUD(Style, StyleRead, StyleCreate, StyleUpdate)


async def get_count(
    response: Response,
    filter: str = Query(None),
    range: str = Query(None),
    sort: str = Query(None),
    session: AsyncSession = Depends(get_session),
):
    count = await crud.get_total_count(
        response=response,
        sort=sort,
        range=range,
        filter=filter,
        session=session,
    )

    return count


async def get_all(
    filter: str = Query(None),
    sort: str = Query(None),
    range: str = Query(None),
    session: AsyncSession = Depends(get_session),
):

    res = await crud.get_model_data(
        sort=sort,
        range=range,
        filter=filter,
        session=session,
    )

    return res


async def get_one(
    style_id: UUID,
    session: AsyncSession = Depends(get_session),
) -> StyleRead:
    res = await crud.get_model_by_id(model_id=style_id, session=session)

    if not res:
        raise HTTPException(
            status_code=404, detail=f"ID: {style_id} not found"
        )

    return res


async def create_one(
    data: StyleCreate,
    session: AsyncSession = Depends(get_session),
) -> Style:
    """Create a single style

    To be used in both create one and create many endpoints
    """

    obj = Style.model_validate(data)

    session.add(obj)

    await session.commit()
    await session.refresh(obj)

    return obj


async def update_one(
    style_id: UUID,
    style_update: StyleUpdate,
    session: AsyncSession = Depends(get_session),
) -> Style:
    """Update a single style"""

    obj = await crud.get_model_by_id(model_id=style_id, session=session)

    update_data = style_update.model_dump(exclude_unset=True)

    obj.sqlmodel_update(update_data)

    session.add(obj)
    await session.commit()
    await session.refresh(obj)

    obj = await get_one(style_id, session)

    return obj


async def delete_one(
    style_id: UUID,
    session: AsyncSession = Depends(get_session),
    s3: aioboto3.Session = Depends(get_s3),
) -> UUID:
    """Delete a single style"""

    obj = await crud.get_model_by_id(model_id=style_id, session=session)

    if not obj:
        raise HTTPException(
            status_code=404, detail=f"ID: {style_id} not found"
        )

    await session.delete(obj)
    await session.commit()

    return style_id
