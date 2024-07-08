from app.layers.models import (
    Layer,
    LayerCreate,
    LayerRead,
    LayerVariables,
    LayerGroupsRead,
)
from app.db import get_session, AsyncSession
from fastapi import (
    Depends,
    APIRouter,
    Query,
    BackgroundTasks,
)
from uuid import UUID
from app.crud import CRUD
from app.layers.services import (
    get_count,
    get_all,
    get_one,
    create_one,
    update_one,
    crud,
    get_layers,
    create_layers,
)
from typing import Any
from sqlmodel import select

router = APIRouter()


@router.get("/groups", response_model=LayerGroupsRead)
async def get_groups(
    session: AsyncSession = Depends(get_session),
) -> LayerGroupsRead:
    """Get all unique groups
    This endpoint allows the menu to be populated with available keys
    """

    groups = {}
    for group in LayerVariables:
        # Get distinct values for each group
        column = getattr(Layer, group.value)
        res = await session.exec(select(column).distinct())
        groups[group.value] = [row for row in res.all()]

    return groups


@router.get("/{layer_id}", response_model=LayerRead)
async def get_layer(
    obj: CRUD = Depends(get_one),
) -> LayerRead:
    """Get a layer by id"""

    return obj


@router.get("", response_model=list[LayerRead])
async def get_all_layers(
    data: Any = Depends(get_all),
    session: AsyncSession = Depends(get_session),
    total_count: int = Depends(get_count),
) -> list[LayerRead]:
    """Get all Layer data"""

    return data


@router.post("", response_model=LayerRead)
async def create_layer(
    create_obj: LayerCreate,
    background_tasks: BackgroundTasks,
    session: AsyncSession = Depends(get_session),
) -> LayerRead:
    """Creates a layer data record"""

    obj = await create_one(create_obj.model_dump(), session, background_tasks)

    return obj


@router.put("/{layer_id}", response_model=LayerRead)
async def update_layer(
    updated_layer: Layer = Depends(update_one),
) -> LayerRead:
    """Update a layer by id"""

    return updated_layer


@router.delete("/batch", response_model=list[str])
async def delete_batch(
    ids: list[UUID],
    session: AsyncSession = Depends(get_session),
) -> list[str]:
    """Delete by a list of ids"""

    for id in ids:
        obj = await crud.get_model_by_id(model_id=id, session=session)
        if obj:
            await session.delete(obj)

    await session.commit()

    return [str(obj_id) for obj_id in ids]


@router.delete("/{layer_id}")
async def delete_layer(
    layer: LayerRead = Depends(get_one),
    session: AsyncSession = Depends(get_session),
) -> None:
    """Delete a layer by id"""

    await session.delete(layer)
    await session.commit()

    return {"ok": True}
